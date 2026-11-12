mod switch;

use anyhow::{Result, bail};
use clap::Parser;
use crossbeam_channel::{Receiver, TryRecvError};
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use switch::Switch;
use tungstenite::{Message, WebSocket};
use warcry_link::{Downlink, FromHost, SessionId, ToHost, Token, Uplink};

const READ_POLL: Duration = Duration::from_millis(5);
const PEEK: usize = 2_048;

#[derive(Parser)]
#[command(
    name = "warcry-hub",
    about = "Fan out one Warcry desktop to many phones"
)]
struct Cli {
    #[arg(long, default_value_t = 8080)]
    port: u16,
    #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::UNSPECIFIED))]
    bind: IpAddr,
    #[arg(long, env = "WARCRY_HUB_SECRET")]
    secret: String,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warcry_hub=info".into()),
        )
        .init();
    let cli = Cli::parse();
    let secret = Token::from_hex(cli.secret.trim())
        .map_err(|_| anyhow::anyhow!("--secret must be 64 hex characters"))?;
    let listener = TcpListener::bind((cli.bind, cli.port))?;
    let switch = Arc::new(Mutex::new(Switch::new()));
    tracing::info!("hub listening on {}", listener.local_addr()?);
    listener
        .incoming()
        .filter_map(Result::ok)
        .for_each(|stream| {
            let switch = switch.clone();
            let spawned = std::thread::Builder::new()
                .name("hub-client".into())
                .spawn(move || serve(stream, switch, secret));
            drop(spawned.inspect_err(|e| tracing::warn!("cannot serve: {e}")));
        });
    Ok(())
}

fn serve(stream: TcpStream, switch: Arc<Mutex<Switch>>, secret: Token) {
    let peer = stream.peer_addr().ok();
    let served = dispatch(stream, switch, secret);
    drop(served.inspect_err(|e| tracing::debug!("session with {peer:?} ended: {e:#}")));
}

fn dispatch(stream: TcpStream, switch: Arc<Mutex<Switch>>, secret: Token) -> Result<()> {
    let head = sniff(&stream);
    let upgrading = head.to_lowercase().contains("upgrade: websocket");
    let target = request_target(&head);
    match (upgrading, target.split('?').next().unwrap_or("/")) {
        (false, path) => page(stream, path),
        (true, "/host") => host(stream, switch, secret, &target),
        (true, _) => phone(stream, switch),
    }
}

fn sniff(stream: &TcpStream) -> String {
    let mut head = [0u8; PEEK];
    let read = stream.peek(&mut head).unwrap_or(0);
    String::from_utf8_lossy(&head[..read]).into_owned()
}

fn request_target(head: &str) -> String {
    head.split_whitespace().nth(1).unwrap_or("/").to_string()
}

fn page(mut stream: TcpStream, path: &str) -> Result<()> {
    let mut head = [0u8; PEEK];
    let _read = stream.read(&mut head)?;
    stream.write_all(&warcry_relay::assets::serve(path))?;
    stream.flush()?;
    Ok(())
}

fn offered_secret(target: &str) -> Option<Token> {
    target
        .split_once('?')
        .map(|(_, query)| query)
        .into_iter()
        .flat_map(|query| query.split('&'))
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| *key == "s")
        .and_then(|(_, value)| Token::from_hex(value).ok())
}

fn host(stream: TcpStream, switch: Arc<Mutex<Switch>>, secret: Token, target: &str) -> Result<()> {
    let offered = offered_secret(target);
    if !offered.is_some_and(|given| secret.verify(&given)) {
        bail!("host presented no usable secret");
    }
    stream.set_read_timeout(Some(READ_POLL))?;
    let mut socket = tungstenite::accept(stream)?;
    let outbound = switch.lock().expect("switch poisoned").host_arrived();
    tracing::info!("desktop connected");
    say(
        &mut socket,
        &ToHost::Hello {
            hub: "warcry-hub".into(),
            protocol: warcry_link::PROTOCOL,
        },
    )?;
    let pumped = pump_host(&mut socket, &switch, &outbound);
    switch.lock().expect("switch poisoned").host_left();
    tracing::info!("desktop disconnected");
    pumped
}

fn pump_host(
    socket: &mut WebSocket<TcpStream>,
    switch: &Arc<Mutex<Switch>>,
    outbound: &Receiver<ToHost>,
) -> Result<()> {
    loop {
        match socket.read() {
            Err(tungstenite::Error::Io(e)) if would_block(&e) => (),
            Err(e) => return Err(e.into()),
            Ok(Message::Text(text)) => {
                serde_json::from_str::<FromHost>(&text)
                    .map(|frame| switch.lock().expect("switch poisoned").on_host(frame))
                    .ok();
            }
            Ok(Message::Close(_)) => bail!("desktop closed"),
            Ok(_) => (),
        }
        drain(outbound, |frame| say(socket, &frame))?;
    }
}

fn phone(stream: TcpStream, switch: Arc<Mutex<Switch>>) -> Result<()> {
    stream.set_read_timeout(Some(READ_POLL))?;
    let mut socket = tungstenite::accept(stream)?;
    let (session, inbound) = switch.lock().expect("switch poisoned").join();
    report(&switch, session, "connected");
    let pumped = pump_phone(&mut socket, &switch, session, &inbound);
    switch.lock().expect("switch poisoned").leave(session);
    report(&switch, session, "disconnected");
    pumped
}

fn report(switch: &Arc<Mutex<Switch>>, session: SessionId, what: &str) {
    let switch = switch.lock().expect("switch poisoned");
    tracing::info!(
        "phone {session} {what}: {} on the hub, desktop {}",
        switch.phones(),
        match switch.has_host() {
            true => "attached",
            false => "away",
        }
    );
}

fn pump_phone(
    socket: &mut WebSocket<TcpStream>,
    switch: &Arc<Mutex<Switch>>,
    session: SessionId,
    inbound: &Receiver<Downlink>,
) -> Result<()> {
    loop {
        match socket.read() {
            Err(tungstenite::Error::Io(e)) if would_block(&e) => (),
            Err(e) => return Err(e.into()),
            Ok(Message::Text(text)) => {
                let forwarded = serde_json::from_str::<Uplink>(&text).map(|message| {
                    switch
                        .lock()
                        .expect("switch poisoned")
                        .on_phone(session, message)
                });
                if let Ok(false) = forwarded {
                    say(socket, &offline())?
                }
            }
            Ok(Message::Close(_)) => bail!("phone closed"),
            Ok(_) => (),
        }
        drain(inbound, |message| say(socket, &message))?;
    }
}

fn offline() -> Downlink {
    Downlink::Denied {
        reason: warcry_link::Denied::NotAuthenticated,
    }
}

fn drain<T>(source: &Receiver<T>, mut send: impl FnMut(T) -> Result<()>) -> Result<()> {
    loop {
        match source.try_recv() {
            Err(TryRecvError::Empty) => return Ok(()),
            Err(TryRecvError::Disconnected) => bail!("hub closed this session"),
            Ok(item) => send(item)?,
        }
    }
}

fn say<T: serde::Serialize>(socket: &mut WebSocket<TcpStream>, message: &T) -> Result<()> {
    socket.send(Message::text(serde_json::to_string(message)?))?;
    Ok(())
}

fn would_block(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

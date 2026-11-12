use anyhow::{Result, bail};
use crossbeam_channel::{Receiver, Sender, TryRecvError, bounded};
use std::time::Duration;
use tungstenite::{Message, WebSocket, stream::MaybeTlsStream};
use warcry_link::{Downlink, FromHost, Press, ToHost, Token};
use warcry_relay::Rigs;
use warcry_relay::host::Hosted;

const BACKLOG: usize = 256;
const RETRY: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(5);

type Upstream = WebSocket<MaybeTlsStream<std::net::TcpStream>>;

pub fn publish(
    url: String,
    secret: Token,
    token: Token,
    rigs: Rigs,
    presses: Sender<Press>,
) -> Sender<Downlink> {
    let (tx, events) = bounded(BACKLOG);
    std::thread::Builder::new()
        .name("hub".into())
        .spawn(move || {
            loop {
                let attempt = session(&url, secret, token, rigs.clone(), presses.clone(), &events);
                match attempt {
                    Ok(()) => println!("  hub closed the connection, reconnecting"),
                    Err(e) => println!("  hub unreachable ({e:#}), retrying in {:?}", RETRY),
                }
                std::thread::sleep(RETRY);
            }
        })
        .map(drop)
        .unwrap_or_else(|e| println!("could not start the hub publisher: {e}"));
    tx
}

fn session(
    url: &str,
    secret: Token,
    token: Token,
    rigs: Rigs,
    presses: Sender<Press>,
    events: &Receiver<Downlink>,
) -> Result<()> {
    let target = format!("{url}?s={}", secret.to_hex());
    let (mut socket, _) = tungstenite::connect(&target)?;
    read_timeout(&socket)?;
    println!("  published to {url}");
    let mut hosted = Hosted::new(token, rigs, presses);
    loop {
        let heard = inbound(&mut socket, &mut hosted)?;
        let sent = outbound(&mut socket, events)?;
        (!heard && !sent).then(|| std::thread::sleep(POLL));
    }
}

fn read_timeout(socket: &Upstream) -> Result<()> {
    match socket.get_ref() {
        MaybeTlsStream::Plain(stream) => stream.set_read_timeout(Some(POLL))?,
        MaybeTlsStream::Rustls(tls) => tls.get_ref().set_read_timeout(Some(POLL))?,
        _ => bail!("unsupported hub transport"),
    }
    Ok(())
}

fn inbound(socket: &mut Upstream, hosted: &mut Hosted) -> Result<bool> {
    match socket.read() {
        Err(tungstenite::Error::Io(e)) if would_block(&e) => Ok(false),
        Err(e) => Err(e.into()),
        Ok(Message::Text(text)) => {
            let frame = serde_json::from_str::<ToHost>(&text)?;
            reply(socket, hosted, frame)?;
            Ok(true)
        }
        Ok(Message::Close(_)) => bail!("hub closed"),
        Ok(_) => Ok(true),
    }
}

fn reply(socket: &mut Upstream, hosted: &mut Hosted, frame: ToHost) -> Result<()> {
    match frame {
        ToHost::Hello { hub, protocol } => {
            match protocol == warcry_link::PROTOCOL {
                true => println!("  hub {hub} speaks protocol {protocol}"),
                false => bail!(
                    "hub speaks protocol {protocol}, we speak {}",
                    warcry_link::PROTOCOL
                ),
            }
            Ok(())
        }
        ToHost::Joined { session } => {
            hosted.joined(session);
            Ok(())
        }
        ToHost::Left { session } => {
            hosted.left(session);
            Ok(())
        }
        ToHost::Uplink { session, message } => hosted
            .uplink(session, message)
            .map(|answer| say(socket, FromHost::just(session, answer)))
            .transpose()
            .map(drop),
    }
}

fn outbound(socket: &mut Upstream, events: &Receiver<Downlink>) -> Result<bool> {
    match events.try_recv() {
        Err(TryRecvError::Empty) => Ok(false),
        Err(TryRecvError::Disconnected) => bail!("nothing left to publish"),
        Ok(message) => {
            say(socket, FromHost::everyone(message))?;
            Ok(true)
        }
    }
}

fn say(socket: &mut Upstream, frame: FromHost) -> Result<()> {
    socket.send(Message::text(serde_json::to_string(&frame)?))?;
    Ok(())
}

fn would_block(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

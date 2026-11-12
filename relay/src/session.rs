use crate::host::Hosted;
use crate::{Rigs, assets};
use crossbeam_channel::{Receiver, Sender, TryRecvError};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::time::Duration;
use tungstenite::{Message, WebSocket};
use warcry_link::{Downlink, Press, Token, Uplink};

const READ_POLL: Duration = Duration::from_millis(5);
const PEEK: usize = 1_024;
const LOCAL: warcry_link::SessionId = 0;

pub fn serve(
    stream: TcpStream,
    peer: SocketAddr,
    downlink: Receiver<Downlink>,
    presses: Sender<Press>,
    rigs: Rigs,
    token: Token,
    quit: Arc<AtomicBool>,
) {
    let served = match wants_websocket(&stream) {
        true => socket(stream, downlink, presses, rigs, token, quit),
        false => page(stream),
    };
    served
        .err()
        .map(|e| tracing::debug!("session with {peer} ended: {e:#}"));
}

fn wants_websocket(stream: &TcpStream) -> bool {
    let mut head = [0u8; PEEK];
    let read = stream.peek(&mut head).unwrap_or(0);
    String::from_utf8_lossy(&head[..read])
        .to_lowercase()
        .contains("upgrade: websocket")
}

fn page(mut stream: TcpStream) -> anyhow::Result<()> {
    let mut head = [0u8; PEEK];
    let read = stream.read(&mut head)?;
    let request = String::from_utf8_lossy(&head[..read]);
    let path = request
        .split_whitespace()
        .nth(1)
        .and_then(|target| target.split('?').next())
        .unwrap_or("/");
    let response = assets::serve(path);
    stream.write_all(&response)?;
    stream.flush()?;
    Ok(())
}

fn socket(
    stream: TcpStream,
    downlink: Receiver<Downlink>,
    presses: Sender<Press>,
    rigs: Rigs,
    token: Token,
    quit: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    stream.set_read_timeout(Some(READ_POLL))?;
    let mut socket = tungstenite::accept(stream)?;
    let mut hosted = Hosted::new(token, rigs, presses);
    hosted.joined(LOCAL);
    while !quit.load(SeqCst) {
        let inbound = pump(&mut socket, &mut hosted)?;
        let outbound = forward(&mut socket, &downlink, &hosted)?;
        timeout(&mut socket, &hosted)?;
        (!inbound && !outbound).then(|| std::thread::yield_now());
    }
    Ok(())
}

fn pump(socket: &mut WebSocket<TcpStream>, hosted: &mut Hosted) -> anyhow::Result<bool> {
    match socket.read() {
        Err(tungstenite::Error::Io(e)) if would_block(&e) => Ok(false),
        Err(e) => Err(e.into()),
        Ok(Message::Text(text)) => {
            let reply = serde_json::from_str::<Uplink>(&text)
                .ok()
                .and_then(|uplink| hosted.uplink(LOCAL, uplink));
            reply.map(|message| say(socket, &message)).transpose()?;
            Ok(true)
        }
        Ok(Message::Close(_)) => Err(anyhow::anyhow!("client closed")),
        Ok(_) => Ok(true),
    }
}

fn forward(
    socket: &mut WebSocket<TcpStream>,
    downlink: &Receiver<Downlink>,
    hosted: &Hosted,
) -> anyhow::Result<bool> {
    match downlink.try_recv() {
        Err(TryRecvError::Empty) => Ok(false),
        Err(TryRecvError::Disconnected) => Err(anyhow::anyhow!("relay closed")),
        Ok(message) => {
            hosted
                .authenticated(LOCAL)
                .then(|| say(socket, &message))
                .transpose()?;
            Ok(true)
        }
    }
}

fn timeout(socket: &mut WebSocket<TcpStream>, hosted: &Hosted) -> anyhow::Result<()> {
    match hosted.expired(LOCAL) {
        false => Ok(()),
        true => {
            say(
                socket,
                &Downlink::Denied {
                    reason: warcry_link::Denied::HandshakeTimeout,
                },
            )?;
            Err(anyhow::anyhow!("handshake timed out"))
        }
    }
}

fn say(socket: &mut WebSocket<TcpStream>, message: &Downlink) -> anyhow::Result<()> {
    socket.send(Message::text(serde_json::to_string(message)?))?;
    Ok(())
}

fn would_block(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

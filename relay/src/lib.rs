pub mod assets;
pub mod host;
mod session;

use anyhow::{Context, Result};
use crossbeam_channel::{Receiver, Sender, unbounded};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use warcry_link::{Downlink, Press, RigInfo, Token};

pub const DEFAULT_PORT: u16 = 8080;

const ACCEPT_POLL: Duration = Duration::from_millis(20);

pub type Rigs = Arc<dyn Fn() -> Vec<RigInfo> + Send + Sync>;

#[derive(Clone, Debug)]
pub struct Config {
    pub bind: IpAddr,
    pub port: u16,
    pub token: Token,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            bind: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            port: DEFAULT_PORT,
            token: fresh_token(),
        }
    }
}

pub fn fresh_token() -> Token {
    let mut bytes = [0u8; warcry_link::auth::TOKEN_LEN];
    getrandom::fill(&mut bytes).expect("system randomness unavailable");
    Token::from_bytes(bytes)
}

#[derive(Default)]
struct Fanout {
    clients: Vec<Sender<Downlink>>,
}

impl Fanout {
    fn join(&mut self) -> Receiver<Downlink> {
        let (tx, rx) = unbounded();
        self.clients.push(tx);
        rx
    }

    fn publish(&mut self, message: &Downlink) {
        self.clients
            .retain(|client| client.send(message.clone()).is_ok());
    }
}

pub struct Relay {
    quit: Arc<AtomicBool>,
    fanout: Arc<Mutex<Fanout>>,
    presses: Receiver<Press>,
    thread: Option<JoinHandle<()>>,
    pub address: SocketAddr,
    pub token: Token,
}

impl Relay {
    pub fn start(config: Config, rigs: Rigs) -> Result<Relay> {
        let listener = TcpListener::bind((config.bind, config.port))
            .with_context(|| format!("cannot listen on port {}", config.port))?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let quit = Arc::new(AtomicBool::new(false));
        let fanout = Arc::new(Mutex::new(Fanout::default()));
        let (press_tx, presses) = unbounded();
        let thread = std::thread::Builder::new().name("relay".into()).spawn({
            let (quit, fanout, token) = (quit.clone(), fanout.clone(), config.token);
            move || accept(listener, quit, fanout, press_tx, rigs, token)
        })?;
        Ok(Relay {
            quit,
            fanout,
            presses,
            thread: Some(thread),
            address,
            token: config.token,
        })
    }

    pub fn publish(&self, message: Downlink) {
        self.fanout
            .lock()
            .expect("fanout poisoned")
            .publish(&message);
    }

    pub fn presses(&self) -> &Receiver<Press> {
        &self.presses
    }

    pub fn url(&self) -> String {
        format!(
            "http://{}:{}/?t={}",
            lan_address(),
            self.address.port(),
            self.token.to_hex()
        )
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.quit.store(true, SeqCst);
        self.thread.take().map(JoinHandle::join);
    }
}

fn accept(
    listener: TcpListener,
    quit: Arc<AtomicBool>,
    fanout: Arc<Mutex<Fanout>>,
    presses: Sender<Press>,
    rigs: Rigs,
    token: Token,
) {
    tracing::info!("companion relay listening on {:?}", listener.local_addr());
    while !quit.load(SeqCst) {
        match listener.accept() {
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(ACCEPT_POLL),
            Err(e) => tracing::warn!("relay accept failed: {e}"),
            Ok((stream, peer)) => {
                let downlink = fanout.lock().expect("fanout poisoned").join();
                let (presses, rigs, quit) = (presses.clone(), rigs.clone(), quit.clone());
                let spawned = std::thread::Builder::new()
                    .name(format!("relay-{peer}"))
                    .spawn(move || {
                        session::serve(stream, peer, downlink, presses, rigs, token, quit)
                    });
                spawned
                    .err()
                    .map(|e| tracing::warn!("cannot serve {peer}: {e}"));
            }
        }
    }
}

pub fn lan_address() -> IpAddr {
    let probe = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .and_then(|socket| {
            socket.connect((Ipv4Addr::new(192, 168, 0, 1), 9))?;
            socket.local_addr()
        })
        .map(|address| address.ip());
    probe.unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_rigs() -> Rigs {
        Arc::new(Vec::new)
    }

    #[test]
    fn fresh_tokens_differ() {
        assert_ne!(fresh_token(), fresh_token());
    }

    #[test]
    fn binding_an_occupied_port_is_an_error_not_a_panic() {
        let config = Config {
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 0,
            ..Config::default()
        };
        let first = Relay::start(config.clone(), no_rigs()).unwrap();
        let taken = Config {
            port: first.address.port(),
            ..config
        };
        let second = Relay::start(taken, no_rigs());
        assert!(second.is_err(), "expected a bind conflict");
    }

    #[test]
    fn a_relay_stops_when_dropped() {
        let config = Config {
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 0,
            ..Config::default()
        };
        let port = {
            let relay = Relay::start(config.clone(), no_rigs()).unwrap();
            relay.address.port()
        };
        let reused = Relay::start(Config { port, ..config }, no_rigs());
        assert!(reused.is_ok(), "port was not released: {:?}", reused.err());
    }

    #[test]
    fn fanout_drops_departed_clients() {
        let mut fanout = Fanout::default();
        let kept = fanout.join();
        drop(fanout.join());
        fanout.publish(&Downlink::Pong {
            nonce: 1,
            server_ms: 2,
        });
        assert_eq!(fanout.clients.len(), 1);
        assert!(kept.try_recv().is_ok());
    }

    #[test]
    fn url_carries_the_token() {
        let config = Config {
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 0,
            ..Config::default()
        };
        let relay = Relay::start(config, no_rigs()).unwrap();
        assert!(relay.url().contains(&relay.token.to_hex()));
        assert!(relay.url().starts_with("http://"));
    }
}

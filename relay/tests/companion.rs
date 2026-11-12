use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tungstenite::{Message, WebSocket, stream::MaybeTlsStream};
use warcry_link::{
    ActionSlot, Alert, Denied, Downlink, Press, RigInfo, RigKind, RigState, Snr, Token, Uplink,
    now_ms,
};
use warcry_relay::{Config, Relay};

type Client = WebSocket<MaybeTlsStream<TcpStream>>;

fn rig() -> RigInfo {
    RigInfo {
        id: 7,
        kind: RigKind::Window,
        label: "Thrallfisher".into(),
        pid: 4821,
        actions: vec![ActionSlot {
            slot: 0,
            label: "reel".into(),
        }],
        snr: Snr::Enhanced,
        ready: true,
    }
}

fn relay() -> Relay {
    let config = Config {
        bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
        port: 0,
        ..Config::default()
    };
    Relay::start(config, Arc::new(|| vec![rig()])).expect("relay did not start")
}

fn connect(relay: &Relay) -> Client {
    let url = format!("ws://127.0.0.1:{}/ws", relay.address.port());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match tungstenite::connect(&url) {
            Ok((socket, _)) => return socket,
            Err(e) if Instant::now() > deadline => panic!("could not connect: {e}"),
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

fn say(client: &mut Client, message: &Uplink) {
    let text = serde_json::to_string(message).unwrap();
    client.send(Message::text(text)).unwrap();
}

fn hear(client: &mut Client) -> Downlink {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        match client.read() {
            Ok(Message::Text(text)) => return serde_json::from_str(&text).unwrap(),
            Ok(_) => continue,
            Err(e) => panic!("read failed: {e}"),
        }
    }
    panic!("no reply before the deadline")
}

fn authenticate(relay: &Relay, client: &mut Client) {
    say(
        client,
        &Uplink::Auth {
            token: relay.token,
            protocol: warcry_link::PROTOCOL,
            client: "test".into(),
        },
    );
    match hear(client) {
        Downlink::Welcome { rigs, protocol, .. } => {
            assert_eq!(protocol, warcry_link::PROTOCOL);
            assert_eq!(rigs, vec![rig()]);
        }
        other => panic!("expected a welcome, got {other:?}"),
    }
}

fn press(seq: u64) -> Uplink {
    Uplink::Press(Press {
        rig: 7,
        slot: 0,
        seq,
        at_ms: now_ms(),
    })
}

#[test]
fn the_page_is_served_over_plain_http() {
    let relay = relay();
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), relay.address.port());
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.contains("<title>Warcry</title>"));
}

#[test]
fn a_good_token_is_welcomed_and_a_press_reaches_the_desktop() {
    let relay = relay();
    let mut client = connect(&relay);
    authenticate(&relay, &mut client);

    say(&mut client, &press(1));
    match hear(&mut client) {
        Downlink::Pressed { rig, slot, ok, .. } => {
            assert_eq!((rig, slot, ok), (7, 0, true));
        }
        other => panic!("expected an ack, got {other:?}"),
    }
    let delivered = relay
        .presses()
        .recv_timeout(Duration::from_secs(5))
        .expect("press never reached the desktop");
    assert_eq!((delivered.rig, delivered.slot), (7, 0));
}

#[test]
fn a_wrong_token_is_refused() {
    let relay = relay();
    let mut client = connect(&relay);
    say(
        &mut client,
        &Uplink::Auth {
            token: Token::from_bytes([0; 32]),
            protocol: warcry_link::PROTOCOL,
            client: "impostor".into(),
        },
    );
    assert!(matches!(
        hear(&mut client),
        Downlink::Denied {
            reason: Denied::BadToken
        }
    ));
}

#[test]
fn an_unauthenticated_press_is_refused_and_never_reaches_the_desktop() {
    let relay = relay();
    let mut client = connect(&relay);
    say(&mut client, &press(1));
    assert!(matches!(
        hear(&mut client),
        Downlink::Denied {
            reason: Denied::NotAuthenticated
        }
    ));
    assert!(relay.presses().try_recv().is_err());
}

#[test]
fn a_replayed_press_is_refused() {
    let relay = relay();
    let mut client = connect(&relay);
    authenticate(&relay, &mut client);
    say(&mut client, &press(4));
    hear(&mut client);
    say(&mut client, &press(4));
    assert!(matches!(
        hear(&mut client),
        Downlink::Denied {
            reason: Denied::Replay
        }
    ));
}

#[test]
fn a_press_for_an_unoffered_slot_is_refused() {
    let relay = relay();
    let mut client = connect(&relay);
    authenticate(&relay, &mut client);
    say(
        &mut client,
        &Uplink::Press(Press {
            rig: 7,
            slot: 9,
            seq: 1,
            at_ms: now_ms(),
        }),
    );
    assert!(matches!(
        hear(&mut client),
        Downlink::Denied {
            reason: Denied::UnknownSlot
        }
    ));
    assert!(relay.presses().try_recv().is_err());
}

#[test]
fn a_press_for_an_unknown_rig_is_refused() {
    let relay = relay();
    let mut client = connect(&relay);
    authenticate(&relay, &mut client);
    say(
        &mut client,
        &Uplink::Press(Press {
            rig: 250,
            slot: 0,
            seq: 1,
            at_ms: now_ms(),
        }),
    );
    assert!(matches!(
        hear(&mut client),
        Downlink::Denied {
            reason: Denied::UnknownRig
        }
    ));
}

#[test]
fn a_bite_alert_reaches_an_authenticated_client() {
    let relay = relay();
    let mut client = connect(&relay);
    authenticate(&relay, &mut client);
    relay.publish(Downlink::Alert {
        rig: 7,
        alert: Alert::Bite { confidence: 0.97 },
        server_ms: now_ms(),
    });
    match hear(&mut client) {
        Downlink::Alert { rig, alert, .. } => {
            assert_eq!(rig, 7);
            assert_eq!(alert, Alert::Bite { confidence: 0.97 });
        }
        other => panic!("expected a bite, got {other:?}"),
    }
}

#[test]
fn state_updates_reach_the_client() {
    let relay = relay();
    let mut client = connect(&relay);
    authenticate(&relay, &mut client);
    relay.publish(Downlink::State {
        rig: 7,
        state: RigState::Casting { ends_in_ms: 30_000 },
        server_ms: now_ms(),
    });
    match hear(&mut client) {
        Downlink::State { state, .. } => {
            assert_eq!(state, RigState::Casting { ends_in_ms: 30_000 })
        }
        other => panic!("expected a state, got {other:?}"),
    }
}

#[test]
fn an_unauthenticated_client_is_told_nothing_about_the_rigs() {
    let relay = relay();
    let mut client = connect(&relay);
    relay.publish(Downlink::Alert {
        rig: 7,
        alert: Alert::Bite { confidence: 1.0 },
        server_ms: now_ms(),
    });
    assert!(matches!(
        hear(&mut client),
        Downlink::Denied {
            reason: Denied::HandshakeTimeout
        }
    ));
}

#[test]
fn ping_is_answered() {
    let relay = relay();
    let mut client = connect(&relay);
    authenticate(&relay, &mut client);
    say(&mut client, &Uplink::Ping { nonce: 99 });
    assert!(matches!(
        hear(&mut client),
        Downlink::Pong { nonce: 99, .. }
    ));
}

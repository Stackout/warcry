use std::net::{IpAddr, Ipv4Addr, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tungstenite::{Message, WebSocket, stream::MaybeTlsStream};
use warcry::profile::{Command, Profile};
use warcry_link::{Denied, Downlink, Press, RigKind, Token, Uplink, now_ms};
use warcry_rig::companion::{Companion, Fire, Settings, Watch};
use warcry_rig::deck;

type Client = WebSocket<MaybeTlsStream<TcpStream>>;

fn profile() -> Profile {
    let pad = |name: &str| Command {
        name: Some(name.into()),
        pad: true,
        ..Command::default()
    };
    Profile {
        commands: vec![
            Command {
                name: Some("cast".into()),
                ..Command::default()
            },
            pad("heal pet"),
            pad("mount up"),
        ],
        ..Profile::default()
    }
}

fn settings(token: Token) -> Settings {
    Settings {
        pids: Vec::new(),
        bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
        port: 0,
        token,
        trust: false,
        advertise: Some("127.0.0.1".into()),
        sounds: Vec::new(),
        bite: "bobber".into(),
        record: None,
        flavor: PathBuf::from("."),
        hub: None,
    }
}

fn hosted(fire: Fire) -> Companion {
    let profile = profile();
    let extra = Arc::new(move || deck::rig(&profile).into_iter().collect());
    let watch: Watch = Arc::new(|_| {});
    Companion::start(settings(Token::from_bytes([5; 32])), extra, fire, watch)
        .expect("companion did not start")
}

fn connect(companion: &Companion) -> Client {
    let url = format!("ws://127.0.0.1:{}/ws", companion.port());
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
    client
        .send(Message::text(serde_json::to_string(message).unwrap()))
        .unwrap();
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
    panic!("the companion said nothing");
}

fn press(slot: u8, seq: u64) -> Uplink {
    Uplink::Press(Press {
        rig: deck::DECK,
        slot,
        seq,
        at_ms: now_ms(),
    })
}

fn settled(fired: &Arc<Mutex<Vec<Press>>>, want: usize) -> Vec<Press> {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        let seen = fired.lock().expect("fired poisoned").clone();
        if seen.len() >= want {
            return seen;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    fired.lock().expect("fired poisoned").clone()
}

fn recorder() -> (Fire, Arc<Mutex<Vec<Press>>>) {
    let fired: Arc<Mutex<Vec<Press>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = fired.clone();
    let fire: Fire = Arc::new(move |press| {
        sink.lock().expect("fired poisoned").push(press);
        Ok("ran a command".into())
    });
    (fire, fired)
}

#[test]
fn a_phone_is_offered_a_button_for_every_command_marked_as_a_pad() {
    let (fire, _) = recorder();
    let companion = hosted(fire);
    let mut client = connect(&companion);
    say(
        &mut client,
        &Uplink::Auth {
            token: Token::from_bytes([5; 32]),
            protocol: warcry_link::PROTOCOL,
            client: "test".into(),
        },
    );
    let Downlink::Welcome { rigs, .. } = hear(&mut client) else {
        panic!("the companion did not welcome a paired phone");
    };
    let decks: Vec<_> = rigs
        .iter()
        .filter(|rig| rig.kind == RigKind::Deck)
        .collect();
    assert_eq!(decks.len(), 1, "{rigs:?}");
    let labels: Vec<&str> = decks[0]
        .actions
        .iter()
        .map(|action| action.label.as_str())
        .collect();
    assert_eq!(labels, vec!["heal pet", "mount up"]);
}

#[test]
fn pressing_a_deck_button_reaches_the_desktop_once_it_is_paired() {
    let (fire, fired) = recorder();
    let companion = hosted(fire);
    let mut client = connect(&companion);
    say(&mut client, &press(1, 1));
    assert!(
        matches!(
            hear(&mut client),
            Downlink::Denied {
                reason: Denied::NotAuthenticated
            }
        ),
        "an unpaired phone pressed a button"
    );
    say(
        &mut client,
        &Uplink::Auth {
            token: Token::from_bytes([5; 32]),
            protocol: warcry_link::PROTOCOL,
            client: "test".into(),
        },
    );
    assert!(matches!(hear(&mut client), Downlink::Welcome { .. }));
    say(&mut client, &press(1, 2));
    assert!(matches!(
        hear(&mut client),
        Downlink::Pressed {
            ok: true,
            slot: 1,
            ..
        }
    ));
    let seen = settled(&fired, 1);
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].slot, 1);
    assert_eq!(seen[0].rig, deck::DECK);
}

#[test]
fn a_button_that_is_not_on_the_deck_never_runs_anything() {
    let (fire, fired) = recorder();
    let companion = hosted(fire);
    let mut client = connect(&companion);
    say(
        &mut client,
        &Uplink::Auth {
            token: Token::from_bytes([5; 32]),
            protocol: warcry_link::PROTOCOL,
            client: "test".into(),
        },
    );
    assert!(matches!(hear(&mut client), Downlink::Welcome { .. }));
    say(&mut client, &press(9, 1));
    assert!(matches!(
        hear(&mut client),
        Downlink::Denied {
            reason: Denied::UnknownSlot
        }
    ));
    assert!(settled(&fired, 1).is_empty());
}

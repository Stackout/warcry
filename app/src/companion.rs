use crate::{App, Deck, home};
use anyhow::{Context, Result};
use serde::Serialize;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use warcry::actions::{self, Shared};
use warcry::keys::Key;
use warcry::profile::Profile;
use warcry_ears::library::Sound;
use warcry_link::{Alert, Attention, Press, RigId, RigInfo, RigKind};
use warcry_rig::companion::{Companion, Extra, Fire, Seen, Settings, Watch};
use warcry_rig::{deck, find};

const BOBBER: &str = include_str!("../../sounds/bobber.json");
const BITE: &str = "bobber";
const RETRY: std::time::Duration = std::time::Duration::from_secs(15);

pub struct Live {
    pub companion: Companion,
    pub qr: String,
    characters: Vec<String>,
    wanted: Vec<u32>,
    attached: std::time::Instant,
}

#[derive(Clone, Serialize)]
pub struct Notice {
    pub kind: String,
    pub text: String,
    pub urgency: u8,
    pub rig: Option<RigId>,
}

#[derive(Serialize)]
pub struct Seat {
    label: String,
    pid: u32,
    deck: bool,
    buttons: Vec<String>,
}

#[derive(Serialize)]
pub struct View {
    pub running: bool,
    pub characters: Vec<String>,
    pub url: Option<String>,
    pub qr: Option<String>,
    pub port: Option<u16>,
    pub hearing: usize,
    pub deaf: Vec<u32>,
    pub seats: Vec<Seat>,
}

pub fn sounds() -> Vec<Sound> {
    serde_json::from_str(BOBBER).unwrap_or_default()
}

pub fn view(state: &App) -> View {
    let live = state.companion.lock();
    let Some(live) = live.as_ref() else {
        return View {
            running: false,
            characters: Vec::new(),
            url: None,
            qr: None,
            port: None,
            hearing: 0,
            deaf: Vec::new(),
            seats: Vec::new(),
        };
    };
    View {
        running: true,
        characters: live.characters.clone(),
        url: Some(live.companion.url.clone()),
        qr: Some(live.qr.clone()),
        port: Some(live.companion.port()),
        hearing: live.companion.listening(),
        deaf: live.companion.deaf.clone(),
        seats: live.companion.rigs().into_iter().map(seat).collect(),
    }
}

fn seat(rig: RigInfo) -> Seat {
    Seat {
        label: rig.label,
        pid: rig.pid,
        deck: rig.kind == RigKind::Deck,
        buttons: rig.actions.into_iter().map(|action| action.label).collect(),
    }
}

pub fn start(app: &AppHandle) -> Result<()> {
    let state = app.state::<App>();
    let settings = state.config.lock().deck.clone();
    let pids = wow();
    let token = warcry_rig::companion::remembered_token(&home().join("companion.token"))?;
    let extra = pads(app.clone());
    let fire = presses(app.clone(), pids.clone());
    let watch = notices(app.clone());
    let companion = Companion::start(
        Settings {
            pids: pids.clone(),
            names: settings.names.clone(),
            bind: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            port: settings.port,
            token,
            trust: settings.trust,
            advertise: settings.advertise.clone(),
            sounds: sounds(),
            bite: BITE.into(),
            record: None,
            flavor: flavor(&settings),
            hub: None,
        },
        extra,
        fire,
        watch,
    )?;
    let qr = qr(&companion.url);
    tracing::info!(
        "companion on {} hearing {} of {} windows",
        companion.url,
        companion.listening(),
        pids.len()
    );
    *state.companion.lock() = Some(Live {
        companion,
        qr,
        characters: warcry_rig::names::Names::all(&flavor(&settings)),
        wanted: pids,
        attached: std::time::Instant::now(),
    });
    Ok(())
}

pub fn wow() -> Vec<u32> {
    find::wow_processes()
        .map(|found| found.into_iter().map(|found| found.pid).collect())
        .unwrap_or_default()
}

pub fn halt(app: &AppHandle) {
    let live = app.state::<App>().companion.lock().take();
    drop(live);
}

struct Attached<'a> {
    wanted: &'a [u32],
    deaf: bool,
    waited: std::time::Duration,
}

fn why_restart(attached: Option<Attached>, enabled: bool, found: &[u32]) -> Option<&'static str> {
    let Some(attached) = attached else {
        return enabled.then_some("the phone deck is on with no companion");
    };
    (attached.wanted != found)
        .then_some("the game windows changed")
        .or_else(|| {
            (attached.deaf && attached.waited >= RETRY)
                .then_some("a game window could not be heard")
        })
}

pub fn rescan(app: &AppHandle) {
    let state = app.state::<App>();
    let found = wow();
    let reason = {
        let live = state.companion.lock();
        let enabled = state.config.lock().deck.enabled;
        let attached = live.as_ref().map(|live| Attached {
            wanted: &live.wanted,
            deaf: !live.companion.deaf.is_empty(),
            waited: live.attached.elapsed(),
        });
        why_restart(attached, enabled, &found)
    };
    let Some(reason) = reason else {
        return;
    };
    tracing::info!("{reason}, reattaching the companion");
    halt(app);
    let started = start(app);
    drop(app.emit("changed", ()));
    drop(started.inspect_err(|e| tracing::warn!("companion did not reattach: {e:#}")));
}

pub fn stop(app: &AppHandle) {
    let live = app.state::<App>().companion.lock().take();
    live.map(|live| tauri::async_runtime::spawn_blocking(move || drop(live)));
}

pub fn announce(app: &AppHandle) {
    app.state::<App>()
        .companion
        .lock()
        .iter()
        .for_each(|live| live.companion.announce());
}

pub fn flavor(settings: &Deck) -> PathBuf {
    settings
        .wow
        .clone()
        .or_else(|| crate::addon::roots().into_iter().next())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn qr(url: &str) -> String {
    use qrcode::render::svg;
    qrcode::QrCode::new(url.as_bytes())
        .map(|code| {
            code.render()
                .min_dimensions(200, 200)
                .dark_color(svg::Color("#0d0618"))
                .light_color(svg::Color("#ffffff"))
                .build()
        })
        .unwrap_or_default()
}

fn pads(app: AppHandle) -> Extra {
    Arc::new(move || {
        current(&app)
            .as_deref()
            .and_then(deck::rig)
            .into_iter()
            .collect()
    })
}

fn presses(app: AppHandle, pids: Vec<u32>) -> Fire {
    Arc::new(move |press| fired(&app, &pids, press))
}

fn notices(app: AppHandle) -> Watch {
    Arc::new(move |seen| noticed(&app, seen))
}

fn current(app: &AppHandle) -> Option<Arc<Profile>> {
    let state = app.state::<App>();
    let running = state
        .engine
        .try_lock()
        .and_then(|engine| engine.as_ref().map(|engine| engine.shared().profile()));
    running.or_else(|| {
        let path = state.config.lock().profile()?;
        Profile::load(&path).ok().map(Arc::new)
    })
}

fn sharing(app: &AppHandle, profile: &Arc<Profile>) -> Arc<Shared> {
    let running = app
        .state::<App>()
        .engine
        .try_lock()
        .and_then(|engine| engine.as_ref().map(|engine| engine.shared().clone()));
    running.unwrap_or_else(|| Shared::new((**profile).clone()))
}

fn fired(app: &AppHandle, pids: &[u32], press: Press) -> Result<String> {
    let profile = current(app).context("no profile is loaded")?;
    let settings = app.state::<App>().config.lock().deck.clone();
    let at = window_pid(pids, &press);
    match choose(&profile, &settings, &press) {
        Some(index) => Ok(run(app, &profile, index, at)),
        None => tap(app, at),
    }
}

fn window_pid(pids: &[u32], press: &Press) -> Option<u32> {
    (press.rig != deck::DECK)
        .then(|| pids.get(press.rig as usize).copied())
        .flatten()
}

pub fn choose(profile: &Profile, settings: &Deck, press: &Press) -> Option<usize> {
    match press.rig == deck::DECK {
        true => deck::command(profile, press.slot),
        false => fishing(settings, press.slot).and_then(|name| named(profile, &name)),
    }
}

fn fishing(settings: &Deck, slot: warcry_link::Slot) -> Option<String> {
    match slot {
        0 => settings.cast.clone(),
        _ => settings.reel.clone(),
    }
    .filter(|name| !name.trim().is_empty())
}

fn named(profile: &Profile, name: &str) -> Option<usize> {
    profile
        .commands
        .iter()
        .position(|command| command.display_name().eq_ignore_ascii_case(name.trim()))
}

fn run(app: &AppHandle, profile: &Arc<Profile>, index: usize, at: Option<u32>) -> String {
    let name = profile.commands[index].display_name();
    actions::trigger_at(&sharing(app, profile), index, "phone", at, drop);
    name
}

fn tap(app: &AppHandle, pid: Option<u32>) -> Result<String> {
    let settings = app.state::<App>().config.lock().deck.clone();
    let key = Key::parse(&settings.key)?;
    let mut sink = warcry::input::EnigoSink::new()?;
    warcry_rig::companion::tap(key, pid, &mut sink)?;
    Ok(match pid {
        Some(pid) => format!("{key} into pid {pid}"),
        None => format!("{key} into the front window"),
    })
}

fn noticed(app: &AppHandle, seen: Seen) {
    told(seen).into_iter().for_each(|notice| {
        logged(&notice);
        speak(app, &notice);
        drop(app.emit("companion", notice));
    })
}

fn logged(notice: &Notice) {
    match notice.urgency >= 1 {
        true => tracing::warn!("{}", notice.text),
        false => tracing::info!("{}", notice.text),
    }
}

fn speak(app: &AppHandle, notice: &Notice) {
    let settings = app.state::<App>().config.lock().deck.clone();
    (settings.speak && notice.urgency >= 2).then(|| warcry::speech_out::say(&notice.text));
}

fn told(seen: Seen) -> Option<Notice> {
    let notice = |kind: &str, text: String, urgency| Notice {
        kind: kind.into(),
        text,
        urgency,
        rig: None,
    };
    match seen {
        Seen::Attached { pid, rate } => Some(notice(
            "attached",
            format!("listening to pid {pid} at {rate} Hz"),
            0,
        )),
        Seen::Deaf { pid, why } => Some(notice("deaf", format!("cannot hear pid {pid}: {why}"), 1)),
        Seen::Named { rig, label } => Some(Notice {
            rig: Some(rig),
            ..notice("named", format!("window {rig} is {label}"), 0)
        }),
        Seen::Alerted { rig, label, alert } => Some(Notice {
            rig: Some(rig),
            ..notice(kind(&alert), said(&label, &alert), alert.urgency())
        }),
        Seen::Fired { rig, slot, what } => Some(Notice {
            rig: Some(rig),
            ..notice("fired", format!("phone button {slot} ran {what}"), 0)
        }),
        Seen::Refused { rig, slot, why } => Some(Notice {
            rig: Some(rig),
            ..notice(
                "refused",
                format!("phone button {slot} did nothing: {why}"),
                1,
            )
        }),
        Seen::Heard { .. } => None,
    }
}

fn kind(alert: &Alert) -> &'static str {
    match alert {
        Alert::Bite { .. } => "bite",
        Alert::BagsFull => "bags",
        Alert::LureExpiring { .. } | Alert::LureGone => "lure",
        Alert::CastExpired => "lapsed",
        Alert::Caught => "caught",
        Alert::Attention { .. } => "attention",
    }
}

fn said(label: &str, alert: &Alert) -> String {
    match alert {
        Alert::Bite { confidence } => {
            format!("{label}: fish on ({:.0}%)", confidence * 100.0)
        }
        Alert::BagsFull => format!("{label}: bags are full"),
        Alert::LureExpiring { secs_left } => format!("{label}: lure ends in {secs_left}s"),
        Alert::LureGone => format!("{label}: the lure has worn off"),
        Alert::CastExpired => format!("{label}: the cast lapsed"),
        Alert::Caught => format!("{label}: caught"),
        Alert::Attention { reason } => format!("{label}: {}", attention(reason)),
    }
}

fn attention(reason: &Attention) -> &'static str {
    match reason {
        Attention::Whisper => "a whisper is waiting",
        Attention::HealthLow => "health is low",
        Attention::Disconnected => "disconnected",
        Attention::CastFailed => "the cast failed",
        Attention::PlayerNearby => "someone is nearby",
        Attention::Unspecified => "look at the game",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use warcry::profile::Command;

    fn profile() -> Profile {
        let named = |name: &str| Command {
            name: Some(name.into()),
            ..Command::default()
        };
        Profile {
            commands: vec![named("cast"), named("Reel It In"), named("heal pet")],
            ..Profile::default()
        }
    }

    #[test]
    fn a_fishing_button_finds_its_command_whatever_the_case() {
        assert_eq!(named(&profile(), "reel it in"), Some(1));
        assert_eq!(named(&profile(), "  cast  "), Some(0));
        assert_eq!(named(&profile(), "mount up"), None);
    }

    fn pressed(rig: RigId, slot: warcry_link::Slot) -> Press {
        Press {
            rig,
            slot,
            seq: 1,
            at_ms: 0,
        }
    }

    #[test]
    fn a_deck_button_runs_the_command_that_claimed_that_slot() {
        let mut profile = profile();
        profile.commands[2].pad = true;
        let settings = Deck::default();
        assert_eq!(
            choose(&profile, &settings, &pressed(deck::DECK, 0)),
            Some(2)
        );
        assert_eq!(choose(&profile, &settings, &pressed(deck::DECK, 1)), None);
    }

    #[test]
    fn the_fishing_buttons_run_the_commands_the_settings_name() {
        let settings = Deck {
            cast: Some("cast".into()),
            reel: Some("reel it in".into()),
            ..Deck::default()
        };
        let profile = profile();
        assert_eq!(choose(&profile, &settings, &pressed(0, 0)), Some(0));
        assert_eq!(choose(&profile, &settings, &pressed(0, 1)), Some(1));
    }

    #[test]
    fn an_unnamed_fishing_button_falls_back_to_a_bare_keystroke() {
        let settings = Deck {
            cast: None,
            reel: Some("   ".into()),
            ..Deck::default()
        };
        let profile = profile();
        assert_eq!(choose(&profile, &settings, &pressed(0, 0)), None);
        assert_eq!(choose(&profile, &settings, &pressed(0, 1)), None);
    }

    #[test]
    fn a_bite_is_urgent_enough_to_be_spoken() {
        let seen = Seen::Alerted {
            rig: 0,
            label: "Shadowbane".into(),
            alert: Alert::Bite { confidence: 0.91 },
        };
        let notice = told(seen).unwrap();
        assert_eq!(notice.kind, "bite");
        assert_eq!(notice.urgency, 3);
        assert_eq!(notice.text, "Shadowbane: fish on (91%)");
    }

    #[test]
    fn catching_a_fish_is_reported_but_never_shouted() {
        let notice = told(Seen::Alerted {
            rig: 1,
            label: "Shadowbane".into(),
            alert: Alert::Caught,
        })
        .unwrap();
        assert_eq!(notice.urgency, 0);
    }

    #[test]
    fn the_steady_stream_of_heard_sounds_is_not_a_notice() {
        let quiet = told(Seen::Heard {
            rig: 0,
            label: "window".into(),
            event: warcry_rig::Event::Sound {
                name: "bobber".into(),
                score: 0.4,
            },
        });
        assert!(quiet.is_none());
    }

    #[test]
    fn the_bundled_bobber_library_parses_and_is_the_bite_it_claims() {
        let library = sounds();
        assert!(!library.is_empty(), "the embedded sound library is empty");
        assert!(library.iter().any(|sound| sound.name == BITE));
    }

    #[test]
    fn each_window_press_is_aimed_at_that_window_only() {
        let pids = [41951, 42003];
        let press = |rig| Press {
            rig,
            slot: 0,
            seq: 1,
            at_ms: 0,
        };
        assert_eq!(window_pid(&pids, &press(0)), Some(41951));
        assert_eq!(window_pid(&pids, &press(1)), Some(42003));
        assert_eq!(window_pid(&pids, &press(2)), None);
        assert_eq!(window_pid(&pids, &press(deck::DECK)), None);
    }

    #[test]
    fn a_window_that_could_not_be_heard_is_retried_but_not_every_probe() {
        let attached = |deaf, secs| {
            Some(Attached {
                wanted: &[41951],
                deaf,
                waited: std::time::Duration::from_secs(secs),
            })
        };
        assert_eq!(why_restart(attached(false, 600), true, &[41951]), None);
        assert_eq!(why_restart(attached(true, 2), true, &[41951]), None);
        assert_eq!(
            why_restart(attached(true, 20), true, &[41951]),
            Some("a game window could not be heard")
        );
        assert_eq!(
            why_restart(attached(false, 1), true, &[99999]),
            Some("the game windows changed")
        );
        assert_eq!(
            why_restart(None, true, &[]),
            Some("the phone deck is on with no companion")
        );
        assert_eq!(why_restart(None, false, &[]), None);
    }
}

use crate::names::Names;
use crate::record::Recorder;
use crate::{Event, attach};
use anyhow::Result;
use crossbeam_channel::{Receiver, Sender, unbounded};
use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use warcry_ears::capture::Tap;
use warcry_ears::detect::{Observation, Outcome};
use warcry_ears::library::Sound;
use warcry_ears::tone::Beacon;
use warcry_link::{
    ActionSlot, Alert, Attention, Downlink, Press, RigId, RigInfo, RigKind, RigState, Slot, Snr,
    now_ms,
};
use warcry_relay::{Config, Relay, Rigs};

pub const DEBOUNCE: Duration = Duration::from_millis(600);

const POLL: Duration = Duration::from_millis(150);

pub type Watch = Arc<dyn Fn(Seen) + Send + Sync>;
pub type Fire = Arc<dyn Fn(Press) -> Result<String> + Send + Sync>;
pub type Extra = Arc<dyn Fn() -> Vec<RigInfo> + Send + Sync>;
pub type Feed = (Sender<(u32, Event)>, Receiver<(u32, Event)>);
pub type Hub = Box<dyn FnOnce(Rigs, Sender<Press>) -> Sender<Downlink> + Send>;

pub struct Settings {
    pub pids: Vec<u32>,
    pub bind: IpAddr,
    pub port: u16,
    pub token: warcry_link::Token,
    pub trust: bool,
    pub advertise: Option<String>,
    pub sounds: Vec<Sound>,
    pub bite: String,
    pub record: Option<PathBuf>,
    pub flavor: PathBuf,
    pub hub: Option<Hub>,
}

#[derive(Clone, Debug)]
pub enum Seen {
    Attached {
        pid: u32,
        rate: u32,
    },
    Deaf {
        pid: u32,
        why: String,
    },
    Heard {
        rig: RigId,
        label: String,
        event: Event,
    },
    Named {
        rig: RigId,
        label: String,
    },
    Alerted {
        rig: RigId,
        label: String,
        alert: Alert,
    },
    Fired {
        rig: RigId,
        slot: Slot,
        what: String,
    },
    Refused {
        rig: RigId,
        slot: Slot,
        why: String,
    },
}

pub struct Companion {
    relay: Arc<Relay>,
    taps: Vec<Tap>,
    registry: Arc<Mutex<Vec<RigInfo>>>,
    extra: Extra,
    quit: Arc<AtomicBool>,
    pub url: String,
    pub deaf: Vec<u32>,
}

impl Companion {
    pub fn start(
        mut settings: Settings,
        extra: Extra,
        fire: Fire,
        watch: Watch,
    ) -> Result<Companion> {
        let registry = Arc::new(Mutex::new(roster(&settings.pids)));
        let (hold, events) = unbounded();
        let (taps, deaf) = ears(&settings, &hold, &watch);
        let rigs = composed(registry.clone(), extra.clone());
        let relay = Arc::new(serve(&settings, rigs.clone())?);
        let quit = Arc::new(AtomicBool::new(false));
        let also = settings.hub.take().map(|hub| hub(rigs, relay.presser()));
        pressing(relay.clone(), quit.clone(), fire, watch.clone())?;
        hearing(
            (hold, events),
            relay.clone(),
            registry.clone(),
            settings.flavor.clone(),
            also,
            quit.clone(),
            watch,
        )?;
        Ok(Companion {
            url: address(&settings.advertise, &relay, settings.trust),
            relay,
            taps,
            registry,
            extra,
            quit,
            deaf,
        })
    }

    pub fn rigs(&self) -> Vec<RigInfo> {
        let mut all = self.registry.lock().expect("registry poisoned").clone();
        all.extend((self.extra)());
        all
    }

    pub fn port(&self) -> u16 {
        self.relay.address.port()
    }

    pub fn publish(&self, message: Downlink) {
        self.relay.publish(message);
    }

    pub fn announce(&self) {
        self.publish(Downlink::Rigs(self.rigs()));
    }

    pub fn listening(&self) -> usize {
        self.taps.len()
    }
}

impl Drop for Companion {
    fn drop(&mut self) {
        self.quit.store(true, SeqCst);
        self.taps.clear();
    }
}

fn ears(settings: &Settings, hold: &Sender<(u32, Event)>, watch: &Watch) -> (Vec<Tap>, Vec<u32>) {
    let mut taps = Vec::new();
    let mut deaf = Vec::new();
    for pid in &settings.pids {
        let recorder = settings
            .record
            .as_deref()
            .map(Recorder::new)
            .transpose()
            .map_err(anyhow::Error::from);
        let opened = recorder.and_then(|recorder| {
            attach(
                *pid,
                hold.clone(),
                settings.sounds.clone(),
                &settings.bite,
                recorder,
            )
        });
        match opened {
            Ok(tap) => {
                watch(Seen::Attached {
                    pid: *pid,
                    rate: tap.rate,
                });
                taps.push(tap);
            }
            Err(e) => {
                watch(Seen::Deaf {
                    pid: *pid,
                    why: format!("{e:#}"),
                });
                deaf.push(*pid);
            }
        }
    }
    (taps, deaf)
}

fn composed(registry: Arc<Mutex<Vec<RigInfo>>>, extra: Extra) -> Rigs {
    Arc::new(move || {
        let mut all = registry.lock().expect("registry poisoned").clone();
        all.extend(extra());
        all
    })
}

fn serve(settings: &Settings, rigs: Rigs) -> Result<Relay> {
    Relay::start(
        Config {
            bind: settings.bind,
            port: settings.port,
            token: settings.token,
            trust: settings.trust,
        },
        rigs,
    )
}

fn address(advertise: &Option<String>, relay: &Relay, trust: bool) -> String {
    let host = advertise
        .clone()
        .unwrap_or_else(|| warcry_relay::lan_address().to_string());
    let port = relay.address.port();
    match trust {
        true => format!("http://{host}:{port}/"),
        false => format!("http://{host}:{port}/?t={}", relay.token.to_hex()),
    }
}

fn pressing(relay: Arc<Relay>, quit: Arc<AtomicBool>, fire: Fire, watch: Watch) -> Result<()> {
    let presses = relay.presses().clone();
    std::thread::Builder::new()
        .name("companion-press".into())
        .spawn(move || {
            let mut last: HashMap<(RigId, Slot), Instant> = HashMap::new();
            while !quit.load(SeqCst) {
                let Ok(press) = presses.recv_timeout(POLL) else {
                    continue;
                };
                let seen = (press.rig, press.slot);
                let fresh = last
                    .get(&seen)
                    .is_none_or(|when| when.elapsed() >= DEBOUNCE);
                match fresh {
                    false => watch(Seen::Refused {
                        rig: press.rig,
                        slot: press.slot,
                        why: format!("a second press within {DEBOUNCE:?}"),
                    }),
                    true => {
                        last.insert(seen, Instant::now());
                        watch(fired(&fire, press));
                    }
                }
            }
        })?;
    Ok(())
}

fn fired(fire: &Fire, press: Press) -> Seen {
    match fire(press) {
        Ok(what) => Seen::Fired {
            rig: press.rig,
            slot: press.slot,
            what,
        },
        Err(e) => Seen::Refused {
            rig: press.rig,
            slot: press.slot,
            why: format!("{e:#}"),
        },
    }
}

fn hearing(
    channel: Feed,
    relay: Arc<Relay>,
    registry: Arc<Mutex<Vec<RigInfo>>>,
    flavor: PathBuf,
    also: Option<Sender<Downlink>>,
    quit: Arc<AtomicBool>,
    watch: Watch,
) -> Result<()> {
    std::thread::Builder::new()
        .name("companion-events".into())
        .spawn(move || {
            let (_hold, events) = channel;
            let names = Names::scan(&flavor);
            while !quit.load(SeqCst) {
                let Ok((pid, event)) = events.recv_timeout(POLL) else {
                    continue;
                };
                let rig = rig_of(&registry, pid);
                identify(&registry, rig, &event, &names)
                    .into_iter()
                    .for_each(|label| watch(Seen::Named { rig, label }));
                let label = label_of(&registry, rig);
                watch(Seen::Heard {
                    rig,
                    label: label.clone(),
                    event: event.clone(),
                });
                for message in downlinks(rig, &event) {
                    alerted(rig, &label, &message).map(&*watch);
                    relay.publish(message.clone());
                    also.as_ref().map(|hub| hub.try_send(message));
                }
            }
        })?;
    Ok(())
}

fn alerted(rig: RigId, label: &str, message: &Downlink) -> Option<Seen> {
    match message {
        Downlink::Alert { alert, .. } => Some(Seen::Alerted {
            rig,
            label: label.to_string(),
            alert: *alert,
        }),
        _ => None,
    }
}

pub fn roster(pids: &[u32]) -> Vec<RigInfo> {
    pids.iter()
        .enumerate()
        .map(|(index, pid)| RigInfo {
            id: index as RigId,
            kind: RigKind::Window,
            label: format!("Window {}", index + 1),
            pid: *pid,
            actions: vec![
                ActionSlot {
                    slot: 0,
                    label: "cast".into(),
                },
                ActionSlot {
                    slot: 1,
                    label: "reel".into(),
                },
            ],
            snr: Snr::Unknown,
            ready: true,
        })
        .collect()
}

pub fn rig_of(registry: &Arc<Mutex<Vec<RigInfo>>>, pid: u32) -> RigId {
    registry
        .lock()
        .expect("registry poisoned")
        .iter()
        .find(|rig| rig.pid == pid)
        .map(|rig| rig.id)
        .unwrap_or_default()
}

pub fn pid_of(registry: &Arc<Mutex<Vec<RigInfo>>>, rig: RigId) -> Option<u32> {
    registry
        .lock()
        .expect("registry poisoned")
        .iter()
        .find(|entry| entry.id == rig)
        .map(|entry| entry.pid)
}

pub fn label_of(registry: &Arc<Mutex<Vec<RigInfo>>>, rig: RigId) -> String {
    registry
        .lock()
        .expect("registry poisoned")
        .iter()
        .find(|entry| entry.id == rig)
        .map(|entry| entry.label.clone())
        .unwrap_or_else(|| format!("rig {rig}"))
}

fn identify(
    registry: &Arc<Mutex<Vec<RigInfo>>>,
    rig: RigId,
    event: &Event,
    names: &Names,
) -> Option<String> {
    let Event::Beacon(Beacon::Identity(identity)) = event else {
        return None;
    };
    let label = names
        .label(*identity)
        .map(str::to_string)
        .unwrap_or_else(|| format!("character {identity:#05o}"));
    registry
        .lock()
        .expect("registry poisoned")
        .iter_mut()
        .filter(|entry| entry.id == rig)
        .for_each(|entry| entry.label = label.clone());
    Some(label)
}

pub fn downlinks(rig: RigId, event: &Event) -> Vec<Downlink> {
    let state = |state| Downlink::State {
        rig,
        state,
        server_ms: now_ms(),
    };
    let alert = |alert| Downlink::Alert {
        rig,
        alert,
        server_ms: now_ms(),
    };
    match event {
        Event::Observation(Observation::Armed { ends_in_ms }) => {
            vec![state(RigState::Casting {
                ends_in_ms: *ends_in_ms,
            })]
        }
        Event::Observation(Observation::Bite { confidence, .. }) => vec![
            alert(Alert::Bite {
                confidence: *confidence,
            }),
            state(RigState::Bite {
                confidence: *confidence,
                age_ms: 0,
            }),
        ],
        Event::Observation(Observation::Disarmed { outcome }) => disarmed(*outcome, state, alert),
        Event::Beacon(Beacon::BagsFull) => {
            vec![alert(Alert::BagsFull), state(RigState::BagsFull)]
        }
        Event::Beacon(Beacon::LureExpiring) => {
            vec![alert(Alert::LureExpiring { secs_left: 60 })]
        }
        Event::Beacon(Beacon::LureGone) => vec![alert(Alert::LureGone)],
        Event::Beacon(Beacon::Attention) => vec![alert(Alert::Attention {
            reason: Attention::Unspecified,
        })],
        Event::Beacon(_) | Event::Sound { .. } => vec![],
    }
}

fn disarmed(
    outcome: Outcome,
    state: impl Fn(RigState) -> Downlink,
    alert: impl Fn(Alert) -> Downlink,
) -> Vec<Downlink> {
    match outcome {
        Outcome::Caught => vec![alert(Alert::Caught), state(RigState::Caught)],
        Outcome::Expired => vec![alert(Alert::CastExpired), state(RigState::Expired)],
        Outcome::Cancelled => vec![state(RigState::Idle)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Arc<Mutex<Vec<RigInfo>>> {
        Arc::new(Mutex::new(roster(&[4821, 4822])))
    }

    #[test]
    fn each_window_becomes_a_rig_with_a_cast_and_a_reel() {
        let made = roster(&[7, 9]);
        assert_eq!(made.len(), 2);
        assert_eq!(made[1].pid, 9);
        assert_eq!(made[1].id, 1);
        assert_eq!(made[0].kind, RigKind::Window);
        let labels: Vec<&str> = made[0]
            .actions
            .iter()
            .map(|action| action.label.as_str())
            .collect();
        assert_eq!(labels, vec!["cast", "reel"]);
    }

    #[test]
    fn a_pid_maps_to_the_rig_that_owns_it() {
        let registry = registry();
        assert_eq!(rig_of(&registry, 4822), 1);
        assert_eq!(pid_of(&registry, 1), Some(4822));
        assert_eq!(pid_of(&registry, 9), None);
    }

    #[test]
    fn a_bite_both_alerts_and_sets_the_state() {
        let made = downlinks(
            3,
            &Event::Observation(Observation::Bite {
                confidence: 0.9,
                into_cast_ms: 1200,
            }),
        );
        assert!(matches!(
            made.as_slice(),
            [
                Downlink::Alert {
                    rig: 3,
                    alert: Alert::Bite { .. },
                    ..
                },
                Downlink::State {
                    state: RigState::Bite { .. },
                    ..
                }
            ]
        ));
    }

    #[test]
    fn a_heard_sound_on_its_own_tells_the_phone_nothing() {
        let quiet = downlinks(
            0,
            &Event::Sound {
                name: "bobber".into(),
                score: 0.6,
            },
        );
        assert!(quiet.is_empty());
    }
}

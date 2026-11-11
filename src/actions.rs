use crate::fault::{self, Cancelled, Report};
use crate::input::{EnigoSink, InputSink, RecordingSink, Tracked};
use crate::keys::Key;
use crate::profile::{Action, Profile, Switch, Target, TargetMode};
use crate::{speech_out, window};
use anyhow::{Context, Result, anyhow, ensure};
use parking_lot::RwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MIN_HOLD_MS: u64 = 30;
const MAX_DEPTH: u8 = 16;

pub struct Shared {
    profile: RwLock<Arc<Profile>>,
    pub listening: AtomicBool,
    pub ptt_active: AtomicBool,
    epoch: AtomicU64,
    alive: AtomicBool,
}

impl Shared {
    pub fn new(profile: Profile) -> Arc<Self> {
        Arc::new(Self {
            listening: AtomicBool::new(profile.settings.start_listening),
            profile: RwLock::new(Arc::new(profile)),
            ptt_active: AtomicBool::new(false),
            epoch: AtomicU64::new(0),
            alive: AtomicBool::new(true),
        })
    }

    pub fn profile(&self) -> Arc<Profile> {
        self.profile.read().clone()
    }

    pub fn set_profile(&self, profile: Profile) {
        *self.profile.write() = Arc::new(profile);
    }

    pub fn stop_all(&self) {
        self.epoch.fetch_add(1, SeqCst);
    }

    pub fn alive(&self) -> bool {
        self.alive.load(SeqCst)
    }

    pub fn shutdown(&self) {
        self.alive.store(false, SeqCst);
    }

    fn epoch(&self) -> u64 {
        self.epoch.load(SeqCst)
    }
}

pub fn trigger(shared: &Arc<Shared>, index: usize, reason: &str) -> JoinHandle<()> {
    let shared = shared.clone();
    let profile = shared.profile();
    let name = profile.commands[index].display_name();
    tracing::info!("▶ {name}  ({reason})");
    fault::spawn(format!("command {name:?}"), move || {
        execute(&shared, &profile, index)
    })
}

pub fn execute(shared: &Shared, profile: &Profile, index: usize) -> Result<()> {
    let command = &profile.commands[index];
    command.stop_others.then(|| shared.stop_all());
    profile
        .settings
        .speak_confirmations
        .then(|| speech_out::say(&command.display_name()));
    let ctx = Ctx {
        shared,
        profile,
        epoch: shared.epoch(),
        dry: false,
    };
    let target = command
        .target
        .as_ref()
        .or(profile.target.as_ref())
        .filter(|_| !command.no_target);
    match target {
        Some(t) => targeted(ctx, t, &command.actions),
        None => drive(EnigoSink::new()?, ctx, &command.actions),
    }
}

pub fn dry_run(profile: &Profile, index: usize) -> Result<Vec<String>> {
    let shared = Shared::new(profile.clone());
    let ctx = Ctx {
        shared: &shared,
        profile,
        epoch: shared.epoch(),
        dry: true,
    };
    let mut sink = RecordingSink::default();
    let target = profile.commands[index]
        .target
        .as_ref()
        .or(profile.target.as_ref());
    if let Some(t) = target {
        sink.note(format!("target window {} ({:?})", t.describe(), t.mode));
    }
    Runner {
        ctx,
        sink: &mut sink,
        depth: 0,
    }
    .actions(&profile.commands[index].actions)?;
    Ok(sink.events)
}

#[derive(Clone, Copy)]
struct Ctx<'a> {
    shared: &'a Shared,
    profile: &'a Profile,
    epoch: u64,
    dry: bool,
}

fn targeted(ctx: Ctx, target: &Target, actions: &[Action]) -> Result<()> {
    let win = window::find(target)?;
    #[cfg(windows)]
    if target.mode == TargetMode::Background {
        return drive(window::BackgroundSink::new(&win), ctx, actions);
    }
    let previous = window::active().ok().flatten().filter(|p| p.id != win.id);
    window::focus(&win)?;
    let result = drive(EnigoSink::new()?, ctx, actions);
    previous
        .filter(|_| target.mode != TargetMode::Focus)
        .map(|p| window::focus(&p).report("restoring focus"));
    result
}

fn drive<S: InputSink>(sink: S, ctx: Ctx, actions: &[Action]) -> Result<()> {
    let mut tracked = Tracked::new(sink);
    let result = Runner {
        ctx,
        sink: &mut tracked,
        depth: 0,
    }
    .actions(actions);
    tracked.release_all();
    fault::ignore_cancel(result)
}

struct Runner<'a> {
    ctx: Ctx<'a>,
    sink: &'a mut dyn InputSink,
    depth: u8,
}

impl Runner<'_> {
    fn check(&self) -> Result<()> {
        (self.ctx.shared.epoch() == self.ctx.epoch)
            .then_some(())
            .ok_or_else(|| Cancelled.into())
    }

    fn sleep(&mut self, ms: u64) -> Result<()> {
        self.ctx.dry.then(|| self.sink.note(format!("wait {ms}ms")));
        let end = Instant::now() + Duration::from_millis(ms * !self.ctx.dry as u64);
        std::iter::from_fn(|| Some(end.saturating_duration_since(Instant::now())))
            .take_while(|left| !left.is_zero())
            .try_for_each(|left| {
                self.check()?;
                std::thread::sleep(left.min(Duration::from_millis(10)));
                Ok::<_, anyhow::Error>(())
            })?;
        self.check()
    }

    fn tap(&mut self, keys: &[Key], hold_ms: u64) -> Result<()> {
        keys.iter().try_for_each(|k| self.sink.key(*k, true))?;
        self.sleep(hold_ms.max(MIN_HOLD_MS))?;
        keys.iter().rev().try_for_each(|k| self.sink.key(*k, false))
    }

    fn actions(&mut self, actions: &[Action]) -> Result<()> {
        actions.iter().try_for_each(|a| self.action(a))
    }

    fn side_effect(&mut self, label: String, effect: impl FnOnce() -> Result<()>) -> Result<()> {
        match self.ctx.dry {
            true => {
                self.sink.note(label);
                Ok(())
            }
            false => effect(),
        }
    }

    fn action(&mut self, action: &Action) -> Result<()> {
        self.check()?;
        match action {
            Action::Press { key, hold_ms } => self.tap(&[Key::parse(key)?], *hold_ms),
            Action::Combo { keys, hold_ms } => self.tap(&Key::parse_combo(keys)?, *hold_ms),
            Action::KeyDown { key } => self.sink.key(Key::parse(key)?, true),
            Action::KeyUp { key } => self.sink.key(Key::parse(key)?, false),
            Action::Type { text } => self.sink.text(text),
            Action::Click {
                button,
                count,
                hold_ms,
            } => (0..*count).try_for_each(|_| {
                self.sink.button(*button, true)?;
                self.sleep((*hold_ms).max(MIN_HOLD_MS))?;
                self.sink.button(*button, false)?;
                self.sleep(MIN_HOLD_MS)
            }),
            Action::MouseDown { button } => self.sink.button(*button, true),
            Action::MouseUp { button } => self.sink.button(*button, false),
            Action::MouseMove { x, y, relative } => self.sink.move_mouse(*x, *y, *relative),
            Action::Scroll { amount, horizontal } => self.sink.scroll(*amount, *horizontal),
            Action::Wait { ms } => self.sleep(*ms),
            Action::WaitRandom { min_ms, max_ms } => self.sleep(random_between(*min_ms, *max_ms)),
            Action::Repeat {
                times: Some(n),
                actions,
            } => (0..*n).try_for_each(|_| self.actions(actions)),
            Action::Repeat {
                times: None,
                actions,
            } if self.ctx.dry => {
                self.sink.note("repeat until stopped:".into());
                self.actions(actions)
            }
            Action::Repeat {
                times: None,
                actions,
            } => loop {
                self.actions(actions)?;
            },
            Action::Focus {
                title,
                process,
                class,
            } => {
                let t = Target {
                    title: title.clone(),
                    process: process.clone(),
                    class: class.clone(),
                    ..Default::default()
                };
                self.side_effect(format!("focus {}", t.describe()), || {
                    window::focus(&window::find(&t)?)
                })
            }
            Action::Run { program, args } => {
                self.side_effect(format!("run {program} {args:?}"), || {
                    std::process::Command::new(program)
                        .args(args)
                        .spawn()
                        .map(drop)
                        .with_context(|| format!("run {program}"))
                })
            }
            Action::Say { text } => self.side_effect(format!("say {text:?}"), || {
                speech_out::say(text);
                Ok(())
            }),
            Action::Log { message } => {
                tracing::info!("{message}");
                Ok(())
            }
            Action::Execute { command } => {
                ensure!(
                    self.depth < MAX_DEPTH,
                    "execute nested more than {MAX_DEPTH} deep"
                );
                let profile = self.ctx.profile;
                let index = profile
                    .find_command(command)
                    .ok_or_else(|| anyhow!("no command named {command:?}"))?;
                Runner {
                    ctx: self.ctx,
                    sink: &mut *self.sink,
                    depth: self.depth + 1,
                }
                .actions(&profile.commands[index].actions)
            }
            Action::StopAll => self.side_effect("stop all commands".into(), || {
                self.ctx.shared.stop_all();
                Ok(())
            }),
            Action::Listening { state } => {
                let listening = &self.ctx.shared.listening;
                let on = match state {
                    Switch::On => true,
                    Switch::Off => false,
                    Switch::Toggle => !listening.load(SeqCst),
                };
                listening.store(on, SeqCst);
                tracing::info!("listening {}", ["off", "on"][on as usize]);
                Ok(())
            }
        }
    }
}

fn random_between(min: u64, max: u64) -> u64 {
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    min + seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(29) % (max - min + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> Profile {
        Profile::parse(
            r#"
            [[command]]
            name = "cast"
            phrase = "cast"
            actions = [{ do = "click", button = "right" }]

            [[command]]
            phrase = "combo"
            actions = [{ do = "combo", keys = "ctrl+s" }, { do = "execute", command = "cast" }]

            [[command]]
            phrase = "forever"
            actions = [{ do = "repeat", actions = [{ do = "wait", ms = 5 }] }]
            "#,
        )
        .unwrap()
    }

    #[test]
    fn dry_run_records_events() {
        let events = dry_run(&profile(), 1).unwrap();
        assert_eq!(
            events,
            vec![
                "key Ctrl down",
                "key s down",
                "wait 30ms",
                "key s up",
                "key Ctrl up",
                "mouse Right down",
                "wait 30ms",
                "mouse Right up",
                "wait 30ms",
            ]
        );
    }

    #[test]
    fn stop_all_cancels_infinite_repeat() {
        let p = profile();
        let shared = Shared::new(p.clone());
        let ctx = Ctx {
            shared: &shared,
            profile: &p,
            epoch: shared.epoch(),
            dry: false,
        };
        let stopper = {
            let s = shared.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(50));
                s.stop_all();
            })
        };
        let mut sink = Tracked::new(RecordingSink::default());
        let result = Runner {
            ctx,
            sink: &mut sink,
            depth: 0,
        }
        .actions(&p.commands[2].actions);
        stopper.join().unwrap();
        assert!(result.unwrap_err().is::<Cancelled>());
    }

    #[test]
    fn tracked_releases_held_input() {
        let mut sink = Tracked::new(RecordingSink::default());
        sink.key(Key::Shift, true).unwrap();
        sink.button(crate::profile::MouseButton::Right, true)
            .unwrap();
        sink.release_all();
        assert_eq!(
            sink.into_inner().events,
            vec![
                "key Shift down",
                "mouse Right down",
                "key Shift up",
                "mouse Right up"
            ]
        );
    }

    #[test]
    fn random_in_range() {
        (0..100).for_each(|_| assert!((10..=20).contains(&random_between(10, 20))));
    }
}

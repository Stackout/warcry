use crate::actions::{self, Shared};
use crate::audio::{self, Gate, Resampler, Vad};
use crate::fault::{self, Report};
use crate::hotkeys::{self, Event};
use crate::matcher::Matcher;
use crate::models;
use crate::profile::{Profile, PttMode};
use crate::stt::{self, Recognizer};
use anyhow::{Result, anyhow};
use crossbeam_channel::{Receiver, Sender, bounded, select, tick, unbounded};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering::SeqCst;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

pub struct Options {
    pub profile: PathBuf,
    pub model: Option<String>,
    pub device: Option<String>,
    pub voice: bool,
    pub download: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Note {
    Downloading {
        model: String,
        read: u64,
        total: u64,
    },
    Ready {
        profile: String,
        commands: usize,
        voice: bool,
        microphone: Option<String>,
        push_to_talk: Option<String>,
    },
    Heard {
        text: String,
        millis: u64,
        command: Option<String>,
        phrase: Option<String>,
        score: f64,
        fired: bool,
    },
    Fired {
        command: String,
        source: String,
    },
    Listening {
        on: bool,
    },
    Reloaded {
        commands: usize,
    },
    Stopped,
}

pub type Notify = Arc<dyn Fn(Note) + Send + Sync>;

pub struct Engine {
    shared: Arc<Shared>,
    quit: Sender<()>,
    thread: Option<JoinHandle<()>>,
}

impl Engine {
    pub fn start(opts: Options, notify: Notify) -> Result<Engine> {
        let shared = Shared::new(Profile::load(&opts.profile)?);
        let (quit, quit_rx) = bounded(1);
        let (ready_tx, ready_rx) = bounded(1);
        let session_shared = shared.clone();
        let thread = std::thread::Builder::new()
            .name("engine".into())
            .spawn(move || {
                let opened = Session::open(session_shared.clone(), opts, notify);
                let failed = opened.as_ref().err().map(|e| anyhow!("{e:#}"));
                ready_tx.send(failed).unwrap_or_default();
                opened
                    .map(|s| s.run(quit_rx))
                    .unwrap_or_else(|_| session_shared.shutdown());
            })?;
        match ready_rx.recv()? {
            Some(e) => Err(e),
            None => Ok(Engine {
                shared,
                quit,
                thread: Some(thread),
            }),
        }
    }

    pub fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }

    pub fn set_listening(&self, on: bool) {
        self.shared.listening.store(on, SeqCst);
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.quit.try_send(()).unwrap_or_default();
        self.thread.take().map(JoinHandle::join);
    }
}

pub fn run(opts: Options) -> Result<()> {
    let notify: Notify = Arc::new(|note| {
        if let Note::Downloading { read, total, .. } = note {
            eprint!(
                "\r  {:>3}%  {} / {} MB",
                read * 100 / total.max(1),
                read >> 20,
                total >> 20
            );
        }
    });
    let engine = Engine::start(opts, notify)?;
    let (tx, rx) = bounded(1);
    ctrlc::set_handler(move || tx.try_send(()).unwrap_or_default())?;
    tracing::info!("ready. Ctrl+C to quit");
    rx.recv()?;
    drop(engine);
    tracing::info!("bye");
    Ok(())
}

struct Heard {
    text: String,
    millis: u64,
}

struct Session {
    shared: Arc<Shared>,
    path: PathBuf,
    notify: Notify,
    keys: Receiver<Event>,
    heard: Receiver<Heard>,
    matcher: Matcher,
    _mic: Option<audio::Mic>,
}

impl Session {
    fn open(shared: Arc<Shared>, opts: Options, notify: Notify) -> Result<Session> {
        let (heard_tx, heard) = unbounded();
        let mic = opts
            .voice
            .then(|| start_voice(&shared, &opts, &notify, heard_tx))
            .transpose()?;
        let (key_tx, keys) = unbounded();
        hotkeys::spawn(shared.clone(), key_tx);
        let profile = shared.profile();
        let push_to_talk = profile.settings.push_to_talk.clone();
        tracing::info!(
            "profile {:?}: {} commands{}",
            profile.name,
            profile.commands.len(),
            push_to_talk
                .as_deref()
                .map(|k| format!(", hold {k} to talk"))
                .unwrap_or_default()
        );
        if let Some(t) = &profile.target {
            tracing::info!("targeting window {} ({:?})", t.describe(), t.mode);
        }
        notify(Note::Ready {
            profile: profile.name.clone(),
            commands: profile.commands.len(),
            voice: opts.voice,
            microphone: mic.as_ref().map(|m| m.name.clone()),
            push_to_talk,
        });
        notify(Note::Listening {
            on: shared.listening.load(SeqCst),
        });
        Ok(Session {
            matcher: Matcher::build(&profile)?,
            shared,
            path: opts.profile,
            notify,
            keys,
            heard,
            _mic: mic,
        })
    }

    fn run(mut self, quit: Receiver<()>) {
        let ticker = tick(Duration::from_millis(250));
        let mut stamp = modified(&self.path);
        let mut listening = self.shared.listening.load(SeqCst);
        loop {
            select! {
                recv(quit) -> _ => break,
                recv(self.keys) -> event => event.map(|e| self.on_hotkey(e)).unwrap_or_default(),
                recv(self.heard) -> heard => heard.map(|h| self.on_heard(h)).unwrap_or_default(),
                recv(ticker) -> _ => {
                    let latest = modified(&self.path);
                    if let Some(m) = (latest != stamp).then(|| self.reload()).flatten() {
                        self.matcher = m;
                    }
                    stamp = latest;
                    let now = self.shared.listening.load(SeqCst);
                    if now != listening {
                        (self.notify)(Note::Listening { on: now });
                    }
                    listening = now;
                }
            }
        }
        self.shared.stop_all();
        self.shared.shutdown();
        (self.notify)(Note::Stopped);
    }

    fn reload(&self) -> Option<Matcher> {
        let profile = Profile::load(&self.path).report("profile not reloaded")?;
        let matcher = Matcher::build(&profile).report("profile not reloaded")?;
        tracing::info!("reloaded profile ({} commands)", profile.commands.len());
        (self.notify)(Note::Reloaded {
            commands: profile.commands.len(),
        });
        self.shared.set_profile(profile);
        Some(matcher)
    }

    fn fire(&self, index: usize, source: String) {
        let command = self.shared.profile().commands[index].display_name();
        (self.notify)(Note::Fired {
            command,
            source: source.clone(),
        });
        drop(actions::trigger(&self.shared, index, &source));
    }

    fn on_hotkey(&self, event: Event) {
        let ptt = &self.shared.ptt_active;
        match (event, self.shared.profile().settings.push_to_talk_mode) {
            (Event::Command(i), _) => self.fire(i, "hotkey".into()),
            (Event::PushToTalk(down), PttMode::Hold) => ptt.store(down, SeqCst),
            (Event::PushToTalk(true), PttMode::Toggle) => {
                ptt.fetch_xor(true, SeqCst);
            }
            (Event::PushToTalk(false), PttMode::Toggle) => {}
        }
    }

    fn on_heard(&self, heard: Heard) {
        let profile = self.shared.profile();
        let listening = self.shared.listening.load(SeqCst);
        let best = self.matcher.best(&heard.text, |i| {
            listening || profile.commands[i].always_active
        });
        let threshold = profile.settings.min_confidence;
        let fired = best.as_ref().is_some_and(|m| m.score >= threshold);
        (self.notify)(Note::Heard {
            text: heard.text.clone(),
            millis: heard.millis,
            command: best
                .as_ref()
                .map(|m| profile.commands[m.command].display_name()),
            phrase: best.as_ref().map(|m| m.phrase.clone()),
            score: best.as_ref().map(|m| m.score).unwrap_or_default(),
            fired,
        });
        match best {
            Some(m) if fired => self.fire(
                m.command,
                format!("heard {:?}, {:.0}%", heard.text, m.score * 100.0),
            ),
            Some(m) => tracing::info!(
                "heard {:?} ({}ms), closest {:?} at {:.0}% (< {:.0}%)",
                heard.text,
                heard.millis,
                m.phrase,
                m.score * 100.0,
                threshold * 100.0
            ),
            None => tracing::info!("heard {:?} ({}ms)", heard.text, heard.millis),
        }
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn start_voice(
    shared: &Arc<Shared>,
    opts: &Options,
    notify: &Notify,
    tx: Sender<Heard>,
) -> Result<audio::Mic> {
    let settings = shared.profile().settings.clone();
    let spec = opts.model.clone().unwrap_or(settings.model.clone());
    let progress_notify = notify.clone();
    let progress_model = spec.clone();
    let progress = move |read, total| {
        progress_notify(Note::Downloading {
            model: progress_model.clone(),
            read,
            total,
        })
    };
    let model = models::resolve(
        &spec,
        opts.download.then_some(&progress as &dyn Fn(u64, u64)),
    )?;
    let recognizer = stt::load(&model, &settings.language)?;
    tracing::info!("speech model {}", model.display());
    let (raw_tx, raw_rx) = unbounded();
    let mic = audio::open(
        opts.device.as_deref().or(settings.input_device.as_deref()),
        raw_tx,
    )?;
    tracing::info!("microphone {:?} at {} Hz", mic.name, mic.rate);
    let (utterance_tx, utterance_rx) = unbounded();
    let (segment_shared, recognize_shared, rate) = (shared.clone(), shared.clone(), mic.rate);
    fault::spawn("audio", move || {
        segment(&segment_shared, rate, raw_rx, utterance_tx)
    });
    fault::spawn("speech recognition", move || {
        recognize(&recognize_shared, recognizer, utterance_rx, tx)
    });
    Ok(mic)
}

fn gate(shared: &Shared) -> Gate {
    match (
        shared.profile().settings.push_to_talk.is_some(),
        shared.ptt_active.load(SeqCst),
    ) {
        (false, _) => Gate::Auto,
        (true, true) => Gate::Open,
        (true, false) => Gate::Closed,
    }
}

fn segment(shared: &Shared, rate: u32, rx: Receiver<Vec<f32>>, tx: Sender<Vec<f32>>) -> Result<()> {
    let mut resampler = Resampler::new(rate);
    let mut vad = Vad::new(&shared.profile().settings);
    rx.iter()
        .flat_map(|chunk| vad.push(&resampler.process(&chunk), gate(shared)))
        .try_for_each(|utterance| tx.send(utterance))
        .map_err(Into::into)
}

fn recognize(
    shared: &Shared,
    mut recognizer: Box<dyn Recognizer>,
    rx: Receiver<Vec<f32>>,
    tx: Sender<Heard>,
) -> Result<()> {
    rx.iter()
        .filter(|_| shared.alive())
        .filter_map(|utterance| {
            let prompt = Matcher::build(&shared.profile())
                .map(|m| m.prompt(600))
                .unwrap_or_default();
            let started = Instant::now();
            let text = recognizer
                .transcribe(&utterance, &prompt)
                .report("transcription")?;
            Some(Heard {
                text: text.trim().to_string(),
                millis: started.elapsed().as_millis() as u64,
            })
        })
        .filter(|h| !crate::matcher::normalize(&h.text).is_empty())
        .try_for_each(|h| tx.send(h))
        .map_err(Into::into)
}

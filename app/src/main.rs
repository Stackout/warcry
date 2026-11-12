#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::{Context, Result, anyhow, ensure};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent, Wry};
use tauri_plugin_opener::OpenerExt;
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::{Context as LayerContext, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};
use warcry::engine::{Engine, Note, Notify, Options};
use warcry::keys::Key;
use warcry::matcher::Matcher;
use warcry::profile::{Action, Command, Profile, Target, TargetMode};
use warcry::{audio, models, window};

const TRAY_ON: &[u8] = include_bytes!("../icons/tray.png");
const TRAY_OFF: &[u8] = include_bytes!("../icons/tray-paused.png");

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct Config {
    profile: Option<PathBuf>,
    device: Option<String>,
    model: Option<String>,
    auto_start: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            profile: None,
            device: None,
            model: None,
            auto_start: true,
        }
    }
}

fn home() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("warcry")
}

fn profiles() -> Vec<PathBuf> {
    std::fs::read_dir(home())
        .map(|d| {
            d.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "toml"))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        })
        .unwrap_or_default()
}

impl Config {
    fn load() -> Config {
        std::fs::read_to_string(home().join("app.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn save(&self) -> Result<()> {
        std::fs::create_dir_all(home())?;
        std::fs::write(home().join("app.json"), serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    fn profile(&self) -> Option<PathBuf> {
        self.profile
            .clone()
            .filter(|p| p.is_file())
            .or_else(|| profiles().into_iter().next())
    }
}

struct Tray {
    icon: TrayIcon,
    listening: CheckMenuItem<Wry>,
    toggle: MenuItem<Wry>,
}

#[derive(Default)]
struct Probe {
    windows: Vec<warcry::window::WindowInfo>,
    devices: Vec<String>,
}

struct App {
    engine: Mutex<Option<Engine>>,
    config: Mutex<Config>,
    probe: Mutex<Probe>,
    busy: AtomicBool,
    tray: Mutex<Option<Tray>>,
    failure: Mutex<Option<String>>,
}

#[derive(Serialize)]
struct CommandView {
    index: usize,
    name: String,
    phrases: Vec<String>,
    more: usize,
    hotkey: Option<String>,
    actions: usize,
    always_active: bool,
    draft: Draft,
    editable: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Draft {
    name: String,
    phrase: String,
    hotkey: String,
    keys: String,
    window: String,
    mode: String,
    always_active: bool,
}

impl Draft {
    fn of(command: &Command) -> Draft {
        Draft {
            name: command.name.clone().unwrap_or_default(),
            phrase: command.phrase.clone().unwrap_or_default(),
            hotkey: command.hotkey.clone().unwrap_or_default(),
            keys: keys_of(command).unwrap_or_default(),
            window: command
                .target
                .as_ref()
                .and_then(|t| t.title.clone())
                .unwrap_or_default(),
            mode: command
                .target
                .as_ref()
                .map(|t| mode_name(t.mode))
                .unwrap_or_else(|| mode_name(TargetMode::default())),
            always_active: command.always_active,
        }
    }

    fn command(self) -> Result<Command, String> {
        let keys = self.keys.trim();
        if keys.is_empty() {
            return Err("give the command a key to press".into());
        }
        Key::parse_combo(keys).map_err(fail)?;
        let some = |text: String| Some(text.trim().to_string()).filter(|t| !t.is_empty());
        Ok(Command {
            name: some(self.name),
            phrase: some(self.phrase),
            hotkey: some(self.hotkey),
            target: some(self.window).map(|title| Target {
                title: Some(title),
                mode: parse_mode(&self.mode),
                ..Target::default()
            }),
            always_active: self.always_active,
            actions: vec![press(keys)],
            ..Command::default()
        })
    }
}

fn press(keys: &str) -> Action {
    match keys.contains('+') {
        true => Action::Combo {
            keys: keys.to_string(),
            hold_ms: 0,
        },
        false => Action::Press {
            key: keys.to_string(),
            hold_ms: 0,
        },
    }
}

fn keys_of(command: &Command) -> Option<String> {
    match command.actions.as_slice() {
        [Action::Press { key, .. }] => Some(key.clone()),
        [Action::Combo { keys, .. }] => Some(keys.clone()),
        _ => None,
    }
}

fn simple(command: &Command) -> bool {
    keys_of(command).is_some()
        && !command.stop_others
        && !command.no_target
        && command
            .target
            .as_ref()
            .is_none_or(|t| t.process.is_none() && t.class.is_none())
}

fn mode_name(mode: TargetMode) -> String {
    match mode {
        TargetMode::Background => "background",
        TargetMode::Focus => "focus",
        TargetMode::FocusRestore => "focus_restore",
    }
    .into()
}

fn parse_mode(name: &str) -> TargetMode {
    match name {
        "background" => TargetMode::Background,
        "focus" => TargetMode::Focus,
        _ => TargetMode::FocusRestore,
    }
}

#[derive(Serialize)]
struct TargetView {
    description: String,
    mode: String,
    found: Option<String>,
}

#[derive(Serialize)]
struct ModelView {
    name: String,
    about: String,
    downloaded: bool,
}

#[derive(Serialize)]
struct Snapshot {
    running: bool,
    busy: bool,
    listening: bool,
    config: Config,
    profile: Option<PathBuf>,
    profile_name: Option<String>,
    profile_error: Option<String>,
    profiles: Vec<PathBuf>,
    commands: Vec<CommandView>,
    target: Option<TargetView>,
    push_to_talk: Option<String>,
    devices: Vec<String>,
    models: Vec<ModelView>,
    model: String,
    failure: Option<String>,
}

fn command_views(profile: &Profile) -> Vec<CommandView> {
    let matcher = Matcher::build(profile).unwrap_or_default();
    profile
        .commands
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let phrases = matcher.phrases_for(i);
            CommandView {
                index: i,
                name: c.display_name(),
                more: phrases.len().saturating_sub(4),
                phrases: phrases.into_iter().take(4).map(String::from).collect(),
                hotkey: c.hotkey.clone(),
                actions: c.actions.len(),
                always_active: c.always_active,
                draft: Draft::of(c),
                editable: simple(c),
            }
        })
        .collect()
}

#[tauri::command]
fn snapshot(state: State<App>) -> Snapshot {
    let config = state.config.lock().clone();
    let path = config.profile();
    let loaded = path.as_ref().map(|p| Profile::load(p));
    let profile = loaded.as_ref().and_then(|r| r.as_ref().ok());
    let engine = state.engine.lock();
    let model = config
        .model
        .clone()
        .or_else(|| profile.map(|p| p.settings.model.clone()))
        .unwrap_or_else(|| "base.en".into());
    Snapshot {
        running: engine.is_some(),
        busy: state.busy.load(SeqCst),
        listening: engine
            .as_ref()
            .is_some_and(|e| e.shared().listening.load(SeqCst)),
        profile_error: loaded
            .as_ref()
            .and_then(|r| r.as_ref().err())
            .map(|e| format!("{e:#}")),
        profile_name: profile.map(|p| p.name.clone()).filter(|n| !n.is_empty()),
        commands: profile.map(command_views).unwrap_or_default(),
        target: profile.and_then(|p| p.target.as_ref()).map(|t| TargetView {
            description: t.describe(),
            mode: format!("{:?}", t.mode),
            found: state
                .probe
                .lock()
                .windows
                .iter()
                .find(|w| window::matches(w, t))
                .map(|w| w.title.clone()),
        }),
        push_to_talk: profile.and_then(|p| p.settings.push_to_talk.clone()),
        devices: state.probe.lock().devices.clone(),
        models: models::KNOWN
            .iter()
            .map(|(name, about)| ModelView {
                name: name.to_string(),
                about: about.to_string(),
                downloaded: models::downloaded(name),
            })
            .collect(),
        model,
        failure: state.failure.lock().take(),
        profiles: profiles(),
        profile: path,
        config,
    }
}

fn emit_note(app: &AppHandle, note: &Note) {
    app.emit("note", note).unwrap_or_default();
}

fn on_note(app: &AppHandle, note: Note) {
    emit_note(app, &note);
    if matches!(
        note,
        Note::Listening { .. } | Note::Ready { .. } | Note::Stopped
    ) {
        refresh_tray(app);
    }
}

fn launch(app: &AppHandle) -> Result<()> {
    let state = app.state::<App>();
    let previous = state.engine.lock().take();
    drop(previous);
    let config = state.config.lock().clone();
    let profile = config
        .profile()
        .ok_or_else(|| anyhow!("create a profile first"))?;
    let handle = app.clone();
    let notify: Notify = Arc::new(move |note| on_note(&handle, note));
    let options = Options {
        profile,
        model: config.model,
        device: config.device,
        voice: true,
        download: true,
    };
    let engine = Engine::start(options, notify)?;
    *state.engine.lock() = Some(engine);
    Ok(())
}

fn start_engine(app: &AppHandle) -> Result<()> {
    let state = app.state::<App>();
    ensure!(!state.busy.swap(true, SeqCst), "Warcry is already starting");
    refresh_tray(app);
    let result = launch(app);
    state.busy.store(false, SeqCst);
    refresh_tray(app);
    app.emit("changed", ()).unwrap_or_default();
    result
}

fn spawn_start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(e) = start_engine(&app) {
            tracing::error!("starting Warcry: {e:#}");
            let message = format!("Couldn't start: {e:#}");
            *app.state::<App>().failure.lock() = Some(message.clone());
            app.emit("failure", message).unwrap_or_default();
        }
    });
}

fn stop_engine(app: &AppHandle) {
    let previous = app.state::<App>().engine.lock().take();
    drop(previous);
    refresh_tray(app);
    app.emit("changed", ()).unwrap_or_default();
}

fn refresh_tray(app: &AppHandle) {
    let state = app.state::<App>();
    let listening = state
        .engine
        .lock()
        .as_ref()
        .map(|e| e.shared().listening.load(SeqCst));
    let busy = state.busy.load(SeqCst);
    if let Some(tray) = state.tray.lock().as_ref() {
        paint_tray(tray, busy, listening);
    }
}

fn paint_tray(tray: &Tray, busy: bool, listening: Option<bool>) {
    let (label, tooltip, icon) = match (busy, listening) {
        (true, _) => ("Starting…", "Warcry: starting", TRAY_OFF),
        (false, None) => ("Start Warcry", "Warcry: stopped", TRAY_OFF),
        (false, Some(true)) => ("Stop Warcry", "Warcry: listening", TRAY_ON),
        (false, Some(false)) => ("Stop Warcry", "Warcry: paused", TRAY_OFF),
    };
    tray.toggle.set_text(label).unwrap_or_default();
    tray.toggle.set_enabled(!busy).unwrap_or_default();
    tray.listening
        .set_enabled(listening.is_some())
        .unwrap_or_default();
    tray.listening
        .set_checked(listening.unwrap_or(false))
        .unwrap_or_default();
    tray.icon.set_tooltip(Some(tooltip)).unwrap_or_default();
    Image::from_bytes(icon)
        .map(|i| tray.icon.set_icon(Some(i)))
        .unwrap_or(Ok(()))
        .unwrap_or_default();
}

fn show(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        w.show().unwrap_or_default();
        w.unminimize().unwrap_or_default();
        w.set_focus().unwrap_or_default();
    }
}

fn toggle_listening(app: &AppHandle) {
    if let Some(e) = app.state::<App>().engine.lock().as_ref() {
        e.set_listening(!e.shared().listening.load(SeqCst));
    }
    refresh_tray(app);
}

fn on_menu(app: &AppHandle, id: &str) {
    let running = app.state::<App>().engine.lock().is_some();
    match (id, running) {
        ("open", _) => show(app),
        ("listening", _) => toggle_listening(app),
        ("toggle", true) => stop_engine(app),
        ("toggle", false) => spawn_start(app),
        ("quit", _) => {
            stop_engine(app);
            app.exit(0);
        }
        _ => {}
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Warcry", true, None::<&str>)?;
    let listening =
        CheckMenuItem::with_id(app, "listening", "Listening", false, false, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle", "Start Warcry", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Warcry", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &open,
            &PredefinedMenuItem::separator(app)?,
            &listening,
            &toggle,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    let icon = TrayIconBuilder::with_id("warcry")
        .icon(Image::from_bytes(TRAY_OFF)?)
        .tooltip("Warcry")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| on_menu(app, event.id().as_ref()))
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show(tray.app_handle());
            }
        })
        .build(app)?;
    *app.state::<App>().tray.lock() = Some(Tray {
        icon,
        listening,
        toggle,
    });
    Ok(())
}

fn fail(e: anyhow::Error) -> String {
    format!("{e:#}")
}

#[tauri::command]
async fn start(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || start_engine(&app))
        .await
        .map_err(|e| e.to_string())?
        .map_err(fail)
}

#[tauri::command]
fn stop(app: AppHandle) {
    stop_engine(&app);
}

#[tauri::command]
fn set_listening(app: AppHandle, on: bool) {
    if let Some(e) = app.state::<App>().engine.lock().as_ref() {
        e.set_listening(on);
    }
    refresh_tray(&app);
}

#[tauri::command]
fn set_config(app: AppHandle, config: Config) -> Result<(), String> {
    let state = app.state::<App>();
    config.save().map_err(fail)?;
    let restart = {
        let mut current = state.config.lock();
        let changed = current.profile != config.profile
            || current.device != config.device
            || current.model != config.model;
        *current = config;
        changed && state.engine.lock().is_some()
    };
    if restart {
        spawn_start(&app);
    }
    Ok(())
}

#[tauri::command]
fn create_profile(app: AppHandle, example: String) -> Result<(), String> {
    let source = match example.as_str() {
        "starter" => warcry::EXAMPLE_STARTER,
        _ => warcry::EXAMPLE_FISHING,
    };
    let name = |n: u32| match n {
        0 => format!("{example}.toml"),
        n => format!("{example}-{n}.toml"),
    };
    let path = (0..)
        .map(|n| home().join(name(n)))
        .find(|p| !p.exists())
        .unwrap_or_default();
    std::fs::create_dir_all(home())
        .and_then(|_| std::fs::write(&path, source))
        .map_err(|e| e.to_string())?;
    let config = Config {
        profile: Some(path),
        ..app.state::<App>().config.lock().clone()
    };
    set_config(app, config)
}

#[tauri::command]
fn open_profile(app: AppHandle) -> Result<(), String> {
    let path = app
        .state::<App>()
        .config
        .lock()
        .profile()
        .context("no profile yet")
        .map_err(fail)?;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn open_folder(app: AppHandle) -> Result<(), String> {
    std::fs::create_dir_all(home()).map_err(|e| e.to_string())?;
    app.opener()
        .open_path(home().to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn download_model(app: AppHandle, name: String) -> Result<(), String> {
    let result = tauri::async_runtime::spawn_blocking(move || {
        let progress = |read, total| {
            emit_note(
                &app,
                &Note::Downloading {
                    model: name.clone(),
                    read,
                    total,
                },
            )
        };
        models::fetch(&name, &progress).map(drop)
    })
    .await
    .map_err(|e| e.to_string())?;
    result.map_err(fail)
}

fn profile_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.state::<App>()
        .config
        .lock()
        .profile()
        .ok_or_else(|| "no profile selected yet".to_string())
}

fn with_profile(
    app: &AppHandle,
    change: impl FnOnce(&mut Profile) -> Result<(), String>,
) -> Result<(), String> {
    let path = profile_path(app)?;
    let mut profile = Profile::load(&path).map_err(fail)?;
    change(&mut profile)?;
    profile.save(&path).map_err(fail)?;
    drop(app.emit("changed", ()));
    Ok(())
}

#[tauri::command]
fn edit_command(app: AppHandle, index: Option<usize>, draft: Draft) -> Result<(), String> {
    let command = draft.command()?;
    with_profile(&app, move |profile| match index {
        None => {
            profile.commands.push(command);
            Ok(())
        }
        Some(at) => match profile.commands.get_mut(at) {
            None => Err(format!("there is no command {at} to edit")),
            Some(slot) => {
                *slot = command;
                Ok(())
            }
        },
    })
}

#[tauri::command]
fn remove_command(app: AppHandle, index: usize) -> Result<(), String> {
    with_profile(&app, move |profile| match index < profile.commands.len() {
        false => Err(format!("there is no command {index} to remove")),
        true => {
            profile.commands.remove(index);
            Ok(())
        }
    })
}

#[tauri::command]
fn target_windows(state: State<App>) -> Vec<String> {
    let mut titles: Vec<String> = state
        .probe
        .lock()
        .windows
        .iter()
        .map(|found| found.title.clone())
        .filter(|title| !title.trim().is_empty())
        .collect();
    titles.sort();
    titles.dedup();
    titles
}

fn spawn_probe(app: AppHandle) {
    let spawned = std::thread::Builder::new()
        .name("probe".into())
        .spawn(move || {
            loop {
                let windows = window::list().unwrap_or_default();
                let devices = audio::devices().unwrap_or_default();
                *app.state::<App>().probe.lock() = Probe { windows, devices };
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
        });
    spawned
        .err()
        .map(|e| tracing::warn!("cannot probe windows and microphones: {e}"));
}

#[tauri::command]
fn try_phrase(state: State<App>, text: String) -> Result<String, String> {
    let path = state
        .config
        .lock()
        .profile()
        .context("no profile yet")
        .map_err(fail)?;
    let profile = Profile::load(&path).map_err(fail)?;
    let threshold = profile.settings.min_confidence;
    let best = Matcher::build(&profile)
        .map_err(fail)?
        .best(&text, |_| true);
    Ok(best
        .map(|m| {
            let verdict = ["would not fire", "would fire"][(m.score >= threshold) as usize];
            format!(
                "→ {} via “{}” ({:.0}%), {verdict}",
                profile.commands[m.command].display_name(),
                m.phrase,
                m.score * 100.0
            )
        })
        .unwrap_or_else(|| "No voice commands in this profile".into()))
}

#[derive(Clone, Serialize)]
struct LogLine {
    level: String,
    message: String,
}

struct Forward(AppHandle);

struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<S: tracing::Subscriber> Layer<S> for Forward {
    fn on_event(&self, event: &tracing::Event<'_>, _: LayerContext<'_, S>) {
        let mut message = Message(String::new());
        event.record(&mut message);
        let line = LogLine {
            level: event.metadata().level().to_string(),
            message: message.0,
        };
        self.0.emit("log", line).unwrap_or_default();
    }
}

fn init_logging(app: AppHandle) {
    tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("warcry=info,warcry_app=info")),
        )
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .with(Forward(app))
        .try_init()
        .unwrap_or_default();
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| show(app)))
        .plugin(tauri_plugin_opener::init())
        .manage(App {
            engine: Mutex::new(None),
            config: Mutex::new(Config::load()),
            probe: Mutex::new(Probe::default()),
            busy: AtomicBool::new(false),
            tray: Mutex::new(None),
            failure: Mutex::new(None),
        })
        .setup(|app| {
            init_logging(app.handle().clone());
            build_tray(app.handle())?;
            spawn_probe(app.handle().clone());
            let config = app.state::<App>().config.lock().clone();
            if config.auto_start && config.profile().is_some() {
                spawn_start(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                window.hide().unwrap_or_default();
            }
        })
        .invoke_handler(tauri::generate_handler![
            snapshot,
            start,
            stop,
            set_listening,
            set_config,
            create_profile,
            open_profile,
            open_folder,
            download_model,
            try_phrase,
            edit_command,
            remove_command,
            target_windows
        ])
        .run(tauri::generate_context!())
        .expect("failed to start Warcry");
}

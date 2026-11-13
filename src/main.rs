use anyhow::{Context, Result, bail, ensure};
use clap::{Parser, Subcommand};
use std::fmt::Display;
use std::path::{Path, PathBuf};
use tracing_subscriber::EnvFilter;
use warcry::matcher::Matcher;
use warcry::profile::Profile;
use warcry::{actions, audio, engine, models, window};

#[derive(Parser)]
#[command(
    name = "warcry",
    version,
    about = "Speak, and Warcry presses the keys."
)]
struct Cli {
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    #[command(about = "Listen for voice commands and hotkeys")]
    Run {
        profile: Option<PathBuf>,
        #[arg(long, help = "Whisper model name or path to a ggml .bin")]
        model: Option<String>,
        #[arg(long, help = "Microphone name (substring)")]
        device: Option<String>,
        #[arg(long, help = "Hotkeys only, no microphone")]
        no_voice: bool,
        #[arg(long, help = "Fail instead of downloading a missing model")]
        no_download: bool,
    },
    #[command(about = "Run the command that best matches TEXT, as if spoken")]
    Simulate {
        profile: PathBuf,
        text: String,
        #[arg(long, help = "Print the input events instead of sending them")]
        dry_run: bool,
    },
    #[command(about = "Validate a profile and list its commands")]
    Check { profile: Option<PathBuf> },
    #[command(about = "List windows you can target")]
    Windows { filter: Option<String> },
    #[command(about = "List microphones")]
    Devices,
    #[command(about = "Show live microphone levels to tune vad_threshold and mic_gain")]
    Meter {
        #[arg(long, help = "Microphone name (substring)")]
        device: Option<String>,
        #[arg(long, default_value_t = 1.0, help = "Gain to apply while measuring")]
        gain: f32,
        #[arg(long, default_value_t = 20, help = "Seconds to measure")]
        seconds: u64,
    },
    #[command(about = "Transcribe a WAV file and show which command it would trigger")]
    Transcribe {
        wav: PathBuf,
        profile: Option<PathBuf>,
        #[arg(long)]
        model: Option<String>,
    },
    #[command(about = "Manage speech models", subcommand)]
    Model(ModelCmd),
    #[command(about = "Write an example profile to get started")]
    Init {
        path: Option<PathBuf>,
        #[arg(long, default_value = "fishing", value_parser = ["fishing", "starter"])]
        example: String,
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum ModelCmd {
    #[command(about = "Download a whisper model")]
    Download {
        #[arg(default_value = "base.en")]
        name: String,
    },
    #[command(about = "List available models")]
    List,
}

fn print_progress(read: u64, total: u64) {
    eprint!(
        "\r  {:>3}%  {} / {} MB",
        read * 100 / total.max(1),
        read >> 20,
        total >> 20
    );
}

fn meter(device: Option<&str>, gain: f32, seconds: u64) -> Result<()> {
    let (tx, rx) = crossbeam_channel::unbounded();
    let mic = audio::open(device, tx)?;
    let mut resampler = warcry::audio::Resampler::new(mic.rate);
    println!("{} at {} Hz, gain {gain}", mic.name, mic.rate);
    println!("speak a command, then stay quiet. ctrl+c to stop early.\n");
    let until = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
    let mut quiet: Vec<f32> = Vec::new();
    let mut peak = 0.0f32;
    while std::time::Instant::now() < until {
        let Ok(chunk) = rx.recv_timeout(std::time::Duration::from_millis(500)) else {
            continue;
        };
        let samples = resampler.process(&chunk);
        let rms = warcry::audio::rms_of(&samples) * gain;
        peak = peak.max(rms);
        quiet.push(rms);
        let bars = (rms * 300.0) as usize;
        println!("{rms:.5} {:needed$}", "", needed = bars.min(60));
    }
    quiet.sort_by(|a, b| a.total_cmp(b));
    let floor = quiet.get(quiet.len() / 4).copied().unwrap_or(0.0);
    println!(
        "\nnoise floor {floor:.5}   loudest {peak:.5}   ratio {:.1}x",
        peak / floor.max(1e-9)
    );
    println!("suggested vad_threshold = {:.4}", (floor * 2.0).max(0.002));
    println!(
        "suggested mic_gain      = {:.1}",
        (0.08 / peak.max(1e-6)).clamp(1.0, 10.0)
    );
    Ok(())
}

fn print_lines(lines: impl IntoIterator<Item = impl Display>) -> Result<()> {
    lines.into_iter().for_each(|l| println!("{l}"));
    Ok(())
}

fn default_profile() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("warcry")
        .join("profile.toml")
}

fn profile_path(given: Option<PathBuf>) -> Result<PathBuf> {
    let path = given.unwrap_or_else(default_profile);
    ensure!(
        path.is_file(),
        "no profile at {}; create one with `warcry init`",
        path.display()
    );
    Ok(path)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let level = ["warcry=info", "warcry=debug", "debug"][cli.verbose.min(2) as usize];
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level)),
        )
        .with_target(false)
        .with_timer(tracing_subscriber::fmt::time::uptime())
        .init();
    match cli.command {
        Cmd::Run {
            profile,
            model,
            device,
            no_voice,
            no_download,
        } => engine::run(engine::Options {
            profile: profile_path(profile)?,
            model,
            device,
            voice: !no_voice,
            download: !no_download,
        }),
        Cmd::Simulate {
            profile,
            text,
            dry_run,
        } => simulate(&profile, &text, dry_run),
        Cmd::Check { profile } => check(&profile_path(profile)?),
        Cmd::Windows { filter } => list_windows(filter),
        Cmd::Devices => print_lines(audio::devices()?),
        Cmd::Meter {
            device,
            gain,
            seconds,
        } => meter(device.as_deref(), gain, seconds),
        Cmd::Transcribe {
            wav,
            profile,
            model,
        } => transcribe(&wav, profile, model),
        Cmd::Model(ModelCmd::Download { name }) => {
            models::fetch(&name, &print_progress).map(|_| eprintln!())
        }
        Cmd::Model(ModelCmd::List) => print_lines(models::KNOWN.iter().map(|(name, about)| {
            let have = ["", "  [downloaded]"][models::path_for(name).is_file() as usize];
            format!("{name:<16}{about}{have}")
        })),
        Cmd::Init {
            path,
            example,
            force,
        } => init(path.unwrap_or_else(default_profile), &example, force),
    }
}

fn simulate(path: &Path, text: &str, dry_run: bool) -> Result<()> {
    let profile = Profile::load(path)?;
    let best = Matcher::build(&profile)?
        .best(text, |_| true)
        .context("profile has no voice commands")?;
    let name = profile.commands[best.command].display_name();
    println!(
        "{text:?} → {name} via {:?} ({:.0}%)",
        best.phrase,
        best.score * 100.0
    );
    ensure!(
        best.score >= profile.settings.min_confidence,
        "below min_confidence {:.0}%, would not fire",
        profile.settings.min_confidence * 100.0
    );
    match dry_run {
        true => print_lines(
            actions::dry_run(&profile, best.command)?
                .iter()
                .map(|e| format!("  {e}")),
        ),
        false => actions::execute(
            &actions::Shared::new(profile.clone()),
            &profile,
            best.command,
        ),
    }
}

fn transcribe(wav: &Path, profile: Option<PathBuf>, model: Option<String>) -> Result<()> {
    let profile = profile
        .map(|p| Profile::load(&p))
        .transpose()?
        .unwrap_or_default();
    let matcher = Matcher::build(&profile)?;
    let model = models::resolve(
        model.as_deref().unwrap_or(&profile.settings.model),
        Some(&print_progress),
    )?;
    let mut recognizer = warcry::stt::load(&model, &profile.settings.language)?;
    let samples = audio::read_wav(wav)?;
    let started = std::time::Instant::now();
    let text = recognizer.transcribe(&samples, &matcher.prompt(600))?;
    println!("{text:?}  ({}ms)", started.elapsed().as_millis());
    let best = matcher.best(&text, |_| true);
    print_lines(best.map(|m| {
        let name = profile.commands[m.command].display_name();
        format!("→ {name} via {:?} ({:.0}%)", m.phrase, m.score * 100.0)
    }))
}

fn check(path: &Path) -> Result<()> {
    let profile = Profile::load(path)?;
    let matcher = Matcher::build(&profile)?;
    println!(
        "{} is valid: {} commands",
        path.display(),
        profile.commands.len()
    );
    profile.commands.iter().enumerate().for_each(|(i, c)| {
        let phrases = matcher.phrases_for(i);
        let hotkey = c
            .hotkey
            .as_deref()
            .map(|h| format!("  [{h}]"))
            .unwrap_or_default();
        println!(
            "  {}{hotkey}  ({} phrasings, {} actions)",
            c.display_name(),
            phrases.len(),
            c.actions.len()
        );
    });
    Ok(())
}

fn list_windows(filter: Option<String>) -> Result<()> {
    let needle = filter.unwrap_or_default().to_lowercase();
    let hit = |w: &window::WindowInfo| {
        [&w.title, &w.process, &w.class]
            .iter()
            .any(|s| s.to_lowercase().contains(&needle))
    };
    println!("{:<24} {:<28} TITLE", "PROCESS", "CLASS");
    window::list()?
        .iter()
        .filter(|w| hit(w))
        .for_each(|w| println!("{:<24} {:<28} {}", w.process, w.class, w.title));
    Ok(())
}

fn init(path: PathBuf, example: &str, force: bool) -> Result<()> {
    let src = match example {
        "starter" => warcry::EXAMPLE_STARTER,
        "fishing" => warcry::EXAMPLE_FISHING,
        other => bail!("unknown example {other:?}"),
    };
    ensure!(
        force || !path.exists(),
        "{} already exists (use --force to overwrite)",
        path.display()
    );
    path.parent().map(std::fs::create_dir_all).transpose()?;
    std::fs::write(&path, src)?;
    println!(
        "wrote {}\nnext: `warcry windows` to find your game, edit [target], then `warcry run`",
        path.display()
    );
    Ok(())
}

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use warcry_ears::detect::{Observation, Outcome};
use warcry_ears::tone::Beacon;
use warcry_link::{ActionSlot, Alert, Attention, Downlink, RigId, RigInfo, RigState, Snr, now_ms};
use warcry_relay::{Config, Relay};
use warcry_rig::{Event, attach, find, names::Names};

#[derive(Parser)]
#[command(
    name = "warcry-rig",
    about = "Listen to game windows and alert the companion"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Windows,
    Analyze {
        wav: Vec<PathBuf>,
    },
    Refine {
        sounds: Vec<PathBuf>,
        #[arg(long)]
        name: String,
        #[arg(long, default_value_t = 0.55)]
        min: f32,
        #[arg(long)]
        out: PathBuf,
    },
    TeachFrom {
        wav: Vec<PathBuf>,
        #[arg(long)]
        name: String,
        #[arg(long, default_value_t = 3.0)]
        from: f32,
        #[arg(long, default_value_t = 1.0)]
        to: f32,
        #[arg(long, default_value = "sounds.json")]
        out: PathBuf,
    },
    Replay {
        wav: Vec<PathBuf>,
        #[arg(long)]
        sounds: PathBuf,
        #[arg(long, default_value = "bobber")]
        bite: String,
    },
    Teach {
        #[arg(long)]
        pid: Option<u32>,
        #[arg(long)]
        name: String,
        #[arg(long, default_value_t = 5)]
        want: usize,
        #[arg(long, default_value = "sounds.json")]
        out: PathBuf,
    },
    Listen {
        #[arg(long)]
        pid: Vec<u32>,
        #[arg(long, default_value = "/Applications/World of Warcraft")]
        wow_root: PathBuf,
        #[arg(long, default_value = "_forever_")]
        flavor: String,
        #[arg(long)]
        serve: bool,
        #[arg(long, default_value_t = warcry_relay::DEFAULT_PORT)]
        port: u16,
        #[arg(long)]
        record: Option<PathBuf>,
        #[arg(long)]
        sounds: Option<PathBuf>,
        #[arg(long, default_value = "bobber")]
        bite: String,
        #[arg(long)]
        bind: Option<std::net::IpAddr>,
        #[arg(long)]
        advertise: Option<String>,
        #[arg(long, default_value = "'")]
        key: String,
        #[arg(long)]
        focus: bool,
        #[arg(long)]
        token_file: Option<PathBuf>,
        #[arg(long)]
        hub: Option<String>,
        #[arg(long, env = "WARCRY_HUB_SECRET")]
        hub_secret: Option<String>,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warcry_rig=info,warcry_relay=info".into()),
        )
        .without_time()
        .init();
    match Cli::parse().command {
        Command::Windows => windows(),
        Command::Analyze { wav } => wav.iter().try_for_each(|path| analyze(path)),
        Command::Refine {
            sounds,
            name,
            min,
            out,
        } => refine(sounds, name, min, out),
        Command::TeachFrom {
            wav,
            name,
            from,
            to,
            out,
        } => teach_from(wav, name, from, to, out),
        Command::Replay { wav, sounds, bite } => replay(wav, sounds, bite),
        Command::Teach {
            pid,
            name,
            want,
            out,
        } => teach(pid, name, want, out),
        Command::Listen {
            pid,
            wow_root,
            flavor,
            serve,
            port,
            record,
            sounds,
            bite,
            bind,
            advertise,
            key,
            focus,
            token_file,
            hub,
            hub_secret,
        } => listen(Listening {
            pids: pid,
            flavor: wow_root.join(flavor),
            serve,
            port,
            record,
            sounds,
            bite,
            bind,
            advertise,
            key,
            focus,
            token_file,
            hub,
            hub_secret,
        }),
    }
}

fn analyze(path: &std::path::Path) -> Result<()> {
    let samples = warcry_rig::record::read_wav(path)?;
    let spectra = warcry_ears::spectro::analyze(&samples);
    let rising = warcry_ears::spectro::flux(&spectra);
    let total = spectra.len();
    println!(
        "\n{}  {total} frames ({:.1}s)",
        path.display(),
        total as f32 * 0.02
    );

    let mut ranked: Vec<(usize, f32)> = rising.iter().copied().enumerate().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut shown: Vec<usize> = Vec::new();
    for (frame, strength) in ranked {
        if shown.iter().any(|seen| frame.abs_diff(*seen) < 10) {
            continue;
        }
        shown.push(frame);
        let at = (frame + 1) as f32 * 0.02;
        let band = spectra[frame + 1]
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.total_cmp(b))
            .map(|(band, _)| band)
            .unwrap_or(0);
        let window = samples
            .get(frame * warcry_ears::FRAME..(frame + 3) * warcry_ears::FRAME)
            .unwrap_or(&[]);
        println!(
            "  {at:6.2}s  {:6.2}s before end  flux {strength:7.1}  band {band:2}  rms {:.4}",
            total as f32 * 0.02 - at,
            warcry_ears::rms(window)
        );
        if shown.len() == 10 {
            break;
        }
    }
    Ok(())
}

const BEACON_MASK: usize = 12;

fn replay(wavs: Vec<PathBuf>, sounds: PathBuf, bite: String) -> Result<()> {
    let library = load_sounds(Some(&sounds))?;
    let target = library
        .iter()
        .find(|sound| sound.name == bite)
        .ok_or_else(|| anyhow::anyhow!("no sound named {bite} in {}", sounds.display()))?
        .clone();
    println!(
        "replaying {} casts against \"{}\" ({} examples, threshold {:.3})\n",
        wavs.len(),
        target.name,
        target.examples(),
        target.threshold
    );
    let mut curious = target.clone();
    curious.threshold = f32::NEG_INFINITY;

    let scored: Vec<(String, f32, f32)> = wavs
        .iter()
        .filter_map(|path| {
            let samples = warcry_rig::record::read_wav(path).ok()?;
            let (best, at) = peak_score(&samples, curious.clone());
            Some((short(path), best, at))
        })
        .collect();

    scored.iter().for_each(|(name, best, at)| {
        let verdict = match *best >= target.threshold {
            true => "HIT ",
            false => "MISS",
        };
        println!("  {verdict} {name:<28} best {best:.3} at {at:5.2}s");
    });

    let hits = scored
        .iter()
        .filter(|(_, best, _)| *best >= target.threshold)
        .count();
    println!("\n  {hits} of {} casts detected", scored.len());
    let mut bests: Vec<f32> = scored.iter().map(|(_, best, _)| *best).collect();
    bests.sort_by(|a, b| a.total_cmp(b));
    bests
        .first()
        .map(|worst| println!("  worst cast scored {worst:.3}"));
    bests.get(bests.len() / 10).map(|tenth| {
        println!(
            "  a threshold of {:.3} would catch 90% of them",
            tenth - 0.01
        )
    });
    Ok(())
}

fn peak_score(samples: &[f32], curious: warcry_ears::library::Sound) -> (f32, f32) {
    let mut library = warcry_ears::library::Library::new(vec![curious]);
    let mut listener = warcry_ears::detect::Listener::new(Default::default());
    listener.cue(warcry_ears::detect::Cue::CastStarted { seconds: 30 });
    samples
        .chunks(warcry_ears::FRAME)
        .filter(|frame| frame.len() == warcry_ears::FRAME)
        .enumerate()
        .flat_map(|(index, frame)| {
            listener.tick();
            library
                .push(frame)
                .into_iter()
                .map(move |heard| (heard.score, index as f32 * 0.02))
                .collect::<Vec<(f32, f32)>>()
        })
        .fold((f32::NEG_INFINITY, 0.0), |best, found| {
            match found.0 > best.0 {
                true => found,
                false => best,
            }
        })
}

fn refine(files: Vec<PathBuf>, name: String, min: f32, out: PathBuf) -> Result<()> {
    let matching: Vec<warcry_ears::library::Sound> = files
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .filter_map(|text| serde_json::from_str::<Vec<warcry_ears::library::Sound>>(&text).ok())
        .flatten()
        .filter(|sound| sound.name == name)
        .collect();
    let flux = matching
        .iter()
        .map(|sound| sound.flux)
        .filter(|flux| *flux > 0.0)
        .fold(f32::NEG_INFINITY, f32::max);
    if !flux.is_finite() {
        bail!("none of those libraries recorded an onset floor; re-run teach-from");
    }
    let pooled: Vec<warcry_ears::library::Patch> = matching
        .into_iter()
        .flat_map(|sound| sound.patches)
        .collect();
    let before = pooled.len();
    let scores = warcry_ears::library::agreement(&pooled);
    println!("pooled {before} examples of \"{name}\"");
    println!(
        "  agreement: {}",
        scores
            .iter()
            .map(|score| format!("{score:.3}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let kept = warcry_ears::library::prune(pooled, min);
    let sound = warcry_ears::library::from_patches(&name, kept, flux)
        .ok_or_else(|| anyhow::anyhow!("pruning at {min} left nothing"))?;
    println!(
        "  kept {} of {before} above {min:.2}, dropped {}",
        sound.examples(),
        before - sound.examples()
    );
    println!("  threshold {:.3}", sound.threshold);
    save(&out, sound)?;
    println!("  wrote {}", out.display());
    Ok(())
}

fn teach_from(wavs: Vec<PathBuf>, name: String, from: f32, to: f32, out: PathBuf) -> Result<()> {
    let frames = warcry_ears::library::DEFAULT_FRAMES;
    println!(
        "looking for \"{name}\" between {from:.1}s and {to:.1}s before the end of each cast\n"
    );
    let clips: Vec<Vec<f32>> = wavs
        .iter()
        .filter_map(|path| {
            let samples = warcry_rig::record::read_wav(path).ok()?;
            let picked = pick(&samples, from, to, frames);
            match &picked {
                None => println!("  {:<28} no usable onset", short(path)),
                Some((at, _)) => println!(
                    "  {:<28} onset at {:.2}s ({:.2}s before end)",
                    short(path),
                    *at as f32 * 0.02,
                    samples.len() as f32 / warcry_ears::RATE as f32 - *at as f32 * 0.02
                ),
            }
            picked.map(|(_, clip)| clip)
        })
        .collect();

    let sound = warcry_ears::library::learn(&name, &clips, frames)
        .ok_or_else(|| anyhow::anyhow!("no examples could be extracted"))?;
    let scores = warcry_ears::library::agreement(&sound.patches);
    println!("\nlearned \"{name}\" from {} examples", sound.examples());
    println!(
        "  agreement: {}",
        scores
            .iter()
            .map(|score| format!("{score:.3}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    println!(
        "  threshold {:.3}  onset floor {:.1}",
        sound.threshold, sound.flux
    );
    save(&out, sound)?;
    println!("  wrote {}", out.display());
    Ok(())
}

fn short(path: &std::path::Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn pick(samples: &[f32], from: f32, to: f32, frames: usize) -> Option<(usize, Vec<f32>)> {
    let spectra = warcry_ears::spectro::analyze(samples);
    let mut rising = warcry_ears::spectro::flux(&spectra);
    beacons(samples).iter().for_each(|at| {
        let low = at.saturating_sub(BEACON_MASK);
        let high = (at + BEACON_MASK).min(rising.len());
        rising.get_mut(low..high).map(|span| span.fill(0.0));
    });
    let total = spectra.len();
    let start = total.checked_sub((from / 0.02) as usize)?;
    let end = total.checked_sub((to / 0.02) as usize)?;
    let onset = rising
        .get(start..end)?
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.total_cmp(b))
        .filter(|(_, strength)| **strength > 0.0)
        .map(|(offset, _)| start + offset + 1)?;
    let head = onset.saturating_sub(2) * warcry_ears::FRAME;
    let tail = ((onset + frames + 3) * warcry_ears::FRAME).min(samples.len());
    Some((onset, samples.get(head..tail)?.to_vec()))
}

fn beacons(samples: &[f32]) -> Vec<usize> {
    let mut decoder = warcry_ears::tone::Decoder::new(Default::default());
    samples
        .chunks(warcry_ears::FRAME)
        .filter(|frame| frame.len() == warcry_ears::FRAME)
        .enumerate()
        .filter_map(|(index, frame)| decoder.push(frame).map(|_| index))
        .collect()
}

const LOOKBACK: f32 = 1.4;
const REACTION_FLOOR: f32 = 0.08;

fn teach(pid: Option<u32>, name: String, want: usize, out: PathBuf) -> Result<()> {
    let pid = pid.map(Ok).unwrap_or_else(sole_window)?;
    let (audio_tx, audio) = crossbeam_channel::unbounded();
    let tap = warcry_ears::capture::open(pid, audio_tx)?;
    let mut resampler = warcry_ears::resample::Resampler::new(tap.rate);
    let ring_len = (warcry_ears::RATE as f32 * (LOOKBACK + 0.5)) as usize;
    let mut ring: std::collections::VecDeque<f32> = std::collections::VecDeque::new();

    let (label_tx, labels) = crossbeam_channel::unbounded();
    std::thread::spawn(move || {
        std::io::stdin()
            .lines()
            .map_while(Result::ok)
            .try_for_each(|_| label_tx.send(()))
            .ok();
    });

    println!("Teaching \"{name}\" from pid {pid} at {} Hz.", tap.rate);
    println!("Fish as normal. The moment you HEAR the sound, press Enter here.");
    println!("Need {want} examples. Ctrl+C to give up.\n");

    let mut examples: Vec<Vec<f32>> = Vec::new();
    while examples.len() < want {
        audio
            .try_recv()
            .map(|samples| {
                ring.extend(resampler.process(&samples));
                let excess = ring.len().saturating_sub(ring_len);
                ring.drain(..excess);
            })
            .ok();
        labels
            .try_recv()
            .map(|()| match region(&ring) {
                None => println!("  not enough audio buffered yet, try again"),
                Some(clip) => {
                    examples.push(clip);
                    println!("  captured example {} of {want}", examples.len());
                }
            })
            .ok();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }

    let sound = warcry_ears::library::learn(&name, &examples, warcry_ears::library::DEFAULT_FRAMES)
        .ok_or_else(|| anyhow::anyhow!("could not build a template from those examples"))?;
    let agreement = warcry_ears::library::agreement(&sound.patches);
    println!("\nlearned \"{name}\" from {} examples", sound.examples());
    println!(
        "  example agreement: {}",
        agreement
            .iter()
            .map(|score| format!("{score:.3}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    println!("  threshold {:.3}", sound.threshold);
    save(&out, sound)?;
    println!("  wrote {}", out.display());
    Ok(())
}

fn region(ring: &std::collections::VecDeque<f32>) -> Option<Vec<f32>> {
    let look = (warcry_ears::RATE as f32 * LOOKBACK) as usize;
    let skip = (warcry_ears::RATE as f32 * REACTION_FLOOR) as usize;
    let end = ring.len().checked_sub(skip)?;
    let start = end.checked_sub(look)?;
    Some(ring.iter().skip(start).take(end - start).copied().collect())
}

fn save(path: &PathBuf, sound: warcry_ears::library::Sound) -> Result<()> {
    let mut sounds: Vec<warcry_ears::library::Sound> = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    sounds.retain(|known| known.name != sound.name);
    sounds.push(sound);
    std::fs::write(path, serde_json::to_string_pretty(&sounds)?)?;
    Ok(())
}

fn sole_window() -> Result<u32> {
    match find::wow_processes()?.as_slice() {
        [only] => Ok(only.pid),
        [] => bail!("no World of Warcraft windows found"),
        many => bail!("{} windows running; pass --pid", many.len()),
    }
}

fn windows() -> Result<()> {
    let found = find::wow_processes()?;
    match found.is_empty() {
        true => println!("no World of Warcraft processes are running"),
        false => found
            .iter()
            .for_each(|candidate| println!("{:>7}  {}", candidate.pid, candidate.command)),
    }
    Ok(())
}

struct Listening {
    pids: Vec<u32>,
    flavor: PathBuf,
    serve: bool,
    port: u16,
    record: Option<PathBuf>,
    sounds: Option<PathBuf>,
    bite: String,
    bind: Option<std::net::IpAddr>,
    advertise: Option<String>,
    key: String,
    focus: bool,
    token_file: Option<PathBuf>,
    hub: Option<String>,
    hub_secret: Option<String>,
}

fn remembered_token(path: Option<&std::path::Path>) -> Result<warcry_link::Token> {
    let Some(path) = path else {
        return Ok(warcry_relay::fresh_token());
    };
    let existing = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| warcry_link::Token::from_hex(text.trim()).ok());
    match existing {
        Some(token) => Ok(token),
        None => {
            let token = warcry_relay::fresh_token();
            path.parent().map(std::fs::create_dir_all).transpose()?;
            std::fs::write(path, token.to_hex())?;
            restrict(path)?;
            println!("wrote a new pairing token to {}", path.display());
            Ok(token)
        }
    }
}

#[cfg(unix)]
fn restrict(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict(_path: &std::path::Path) -> Result<()> {
    Ok(())
}

fn pairing_qr(url: &str) -> Option<String> {
    use qrcode::render::unicode::Dense1x2;
    qrcode::QrCode::new(url.as_bytes()).ok().map(|code| {
        code.render::<Dense1x2>()
            .dark_color(Dense1x2::Light)
            .light_color(Dense1x2::Dark)
            .quiet_zone(true)
            .build()
    })
}

fn load_sounds(path: Option<&std::path::Path>) -> Result<Vec<warcry_ears::library::Sound>> {
    let Some(path) = path else {
        return Ok(Vec::new());
    };
    let text =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    Ok(serde_json::from_str(&text)?)
}

fn listen(options: Listening) -> Result<()> {
    let Listening {
        pids,
        flavor,
        serve,
        port,
        record,
        sounds,
        bite,
        bind,
        advertise,
        key,
        focus,
        token_file,
        hub,
        hub_secret,
    } = options;
    let key = warcry::keys::Key::parse(&key)?;
    let pids = match pids.is_empty() {
        false => pids,
        true => find::wow_processes()?
            .into_iter()
            .map(|candidate| candidate.pid)
            .collect(),
    };
    if pids.is_empty() {
        bail!("no World of Warcraft windows found; start the game or pass --pid");
    }

    let names = Names::scan(&flavor);
    println!(
        "{} characters known from {}",
        names.known(),
        flavor.display()
    );

    let library = load_sounds(sounds.as_deref())?;
    match library.is_empty() {
        true => println!("no sound library loaded; bites will not be detected"),
        false => library.iter().for_each(|sound| {
            println!(
                "sound \"{}\": {} examples, threshold {:.3}{}",
                sound.name,
                sound.examples(),
                sound.threshold,
                match sound.name == bite {
                    true => "  <- reported as a bite",
                    false => "",
                }
            )
        }),
    }

    let registry = Arc::new(Mutex::new(roster(&pids)));
    let (events_tx, events) = crossbeam_channel::unbounded();
    let taps: Vec<_> = pids
        .iter()
        .map(|pid| {
            let recorder = record
                .as_deref()
                .map(warcry_rig::record::Recorder::new)
                .transpose()?;
            attach(*pid, events_tx.clone(), library.clone(), &bite, recorder)
        })
        .collect::<Result<_>>()?;
    record
        .as_deref()
        .map(|dir| println!("recording armed casts to {}", dir.display()));
    taps.iter()
        .zip(&pids)
        .for_each(|(tap, pid)| println!("attached pid {pid} at {} Hz", tap.rate));

    let token = remembered_token(token_file.as_deref())?;
    let relay = serve
        .then(|| {
            let rigs = registry.clone();
            Relay::start(
                Config {
                    port,
                    bind: bind.unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)),
                    token,
                },
                Arc::new(move || rigs.lock().expect("registry poisoned").clone()),
            )
        })
        .transpose()?;
    relay.as_ref().map(|relay| {
        let url = advertise
            .as_deref()
            .map(|host| {
                format!(
                    "http://{host}:{}/?t={}",
                    relay.address.port(),
                    relay.token.to_hex()
                )
            })
            .unwrap_or_else(|| relay.url());
        pairing_qr(&url).map(|qr| println!("\nscan this with your iPhone camera:\n{qr}"));
        println!("or type it: {url}\n");
    });

    let (press_tx, press_rx) = crossbeam_channel::unbounded();
    relay.as_ref().map(|relay| {
        let (from_relay, into_shared) = (relay.presses().clone(), press_tx.clone());
        std::thread::Builder::new()
            .name("relay-presses".into())
            .spawn(move || {
                from_relay
                    .iter()
                    .try_for_each(|press| into_shared.send(press))
                    .ok();
            })
    });

    let published = hub
        .map(|url| {
            let secret = hub_secret
                .as_deref()
                .map(str::trim)
                .map(warcry_link::Token::from_hex)
                .transpose()
                .map_err(|_| anyhow::anyhow!("--hub-secret must be 64 hex characters"))?
                .ok_or_else(|| anyhow::anyhow!("--hub needs --hub-secret"))?;
            let rigs = registry.clone();
            println!("publishing to {url}");
            Ok::<_, anyhow::Error>(warcry_rig::hub::publish(
                url,
                secret,
                token,
                Arc::new(move || rigs.lock().expect("registry poisoned").clone()),
                press_tx.clone(),
            ))
        })
        .transpose()?;

    let titles = focus.then(|| window_titles(&pids)).unwrap_or_default();
    println!(
        "phone buttons send {key}{}",
        match focus {
            true => " after focusing the game window",
            false => " to whatever window is in front",
        }
    );
    match accessibility_granted() {
        true => println!("accessibility: granted"),
        false => {
            println!("accessibility: DENIED -- keystrokes will go nowhere.");
            println!("  System Settings > Privacy & Security > Accessibility, add the app you");
            println!("  launched this from (Terminal, iTerm, VS Code), then run this again.");
        }
    }
    bridge(press_rx, registry.clone(), key, titles);

    println!("listening. Ctrl+C to stop.");
    while let Ok((pid, event)) = events.recv() {
        let rig = rig_of(&registry, pid);
        let label = label_of(&registry, rig);
        report(&label, &event);
        identify(&registry, rig, &event, &names);
        for message in downlinks(rig, &event) {
            relay.as_ref().map(|relay| relay.publish(message.clone()));
            published.as_ref().map(|hub| hub.try_send(message));
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn accessibility_granted() -> bool {
    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }
    unsafe { AXIsProcessTrusted() }
}

#[cfg(not(target_os = "macos"))]
fn accessibility_granted() -> bool {
    true
}

fn window_titles(pids: &[u32]) -> std::collections::HashMap<u32, String> {
    let listed = warcry::window::list();
    let found = match listed {
        Err(e) => {
            println!("WARNING cannot list windows, so --focus cannot work: {e:#}");
            println!("        grant your terminal Automation access to System Events");
            Vec::new()
        }
        Ok(found) => found,
    };
    let titles: std::collections::HashMap<u32, String> = found
        .into_iter()
        .map(|window| (window.pid, window.title))
        .collect();
    pids.iter().for_each(|pid| match titles.get(pid) {
        Some(title) => println!("focus target for pid {pid}: {title:?}"),
        None => println!("WARNING pid {pid} has no window, so --focus will be skipped for it"),
    });
    titles
}

fn bridge(
    presses: crossbeam_channel::Receiver<warcry_link::Press>,
    registry: Arc<Mutex<Vec<RigInfo>>>,
    key: warcry::keys::Key,
    titles: std::collections::HashMap<u32, String>,
) {
    std::thread::spawn(move || {
        let mut sink = match warcry::input::EnigoSink::new() {
            Ok(sink) => sink,
            Err(e) => {
                println!("input unavailable, phone presses will do nothing: {e:#}");
                return;
            }
        };
        let mut last: std::collections::HashMap<RigId, std::time::Instant> =
            std::collections::HashMap::new();
        while let Ok(press) = presses.recv() {
            let fresh = last
                .get(&press.rig)
                .is_none_or(|when| when.elapsed() >= DEBOUNCE);
            if !fresh {
                println!(
                    "  ignored a second press on rig {} within {DEBOUNCE:?}",
                    press.rig
                );
                continue;
            }
            last.insert(press.rig, std::time::Instant::now());
            let pid = pid_of(&registry, press.rig);
            let target = pid.zip(titles.get(&pid.unwrap_or(0)).cloned());
            let focusing = match &target {
                Some((pid, title)) => format!("focus pid {pid} {title:?}, "),
                None => "no focus, ".to_string(),
            };
            let sent = deliver(&mut sink, key, target);
            match sent {
                Ok(()) => println!(
                    "  {focusing}sent {key} for rig {} slot {}",
                    press.rig, press.slot
                ),
                Err(e) => println!("  {focusing}could not send {key}: {e:#}"),
            }
        }
    });
}

const SETTLE: std::time::Duration = std::time::Duration::from_millis(120);
const HOLD: std::time::Duration = std::time::Duration::from_millis(45);
const DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(600);

fn deliver(
    sink: &mut impl warcry::input::InputSink,
    key: warcry::keys::Key,
    target: Option<(u32, String)>,
) -> Result<()> {
    target
        .map(|(pid, _)| raise(pid))
        .transpose()?
        .map(|()| std::thread::sleep(SETTLE));
    sink.key(key, true)?;
    std::thread::sleep(HOLD);
    sink.key(key, false)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn raise(pid: u32) -> Result<()> {
    let script = format!(
        "tell application \"System Events\" to set frontmost of (first process whose unix id is {pid}) to true"
    );
    let out = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()?;
    match out.status.success() {
        true => Ok(()),
        false => bail!(
            "could not raise pid {pid}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ),
    }
}

#[cfg(not(target_os = "macos"))]
fn raise(_pid: u32) -> Result<()> {
    Ok(())
}

fn pid_of(registry: &Arc<Mutex<Vec<RigInfo>>>, rig: RigId) -> Option<u32> {
    registry
        .lock()
        .expect("registry poisoned")
        .iter()
        .find(|entry| entry.id == rig)
        .map(|entry| entry.pid)
}

fn roster(pids: &[u32]) -> Vec<RigInfo> {
    pids.iter()
        .enumerate()
        .map(|(index, pid)| RigInfo {
            id: index as RigId,
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

fn rig_of(registry: &Arc<Mutex<Vec<RigInfo>>>, pid: u32) -> RigId {
    registry
        .lock()
        .expect("registry poisoned")
        .iter()
        .find(|rig| rig.pid == pid)
        .map(|rig| rig.id)
        .unwrap_or_default()
}

fn label_of(registry: &Arc<Mutex<Vec<RigInfo>>>, rig: RigId) -> String {
    registry
        .lock()
        .expect("registry poisoned")
        .iter()
        .find(|entry| entry.id == rig)
        .map(|entry| entry.label.clone())
        .unwrap_or_else(|| format!("rig {rig}"))
}

fn identify(registry: &Arc<Mutex<Vec<RigInfo>>>, rig: RigId, event: &Event, names: &Names) {
    let Event::Beacon(Beacon::Identity(identity)) = event else {
        return;
    };
    let identity = *identity;
    let label = names
        .label(identity)
        .map(str::to_string)
        .unwrap_or_else(|| format!("character {identity:#05o}"));
    println!("  rig {rig} is {label}");
    registry
        .lock()
        .expect("registry poisoned")
        .iter_mut()
        .filter(|entry| entry.id == rig)
        .for_each(|entry| entry.label = label.clone());
}

fn report(label: &str, event: &Event) {
    match event {
        Event::Beacon(beacon) => println!("[{label}] beacon {beacon:?}"),
        Event::Observation(Observation::Bite {
            confidence,
            into_cast_ms,
        }) => println!(
            "[{label}] BITE  confidence {confidence:.2}  {:.1}s into the cast",
            *into_cast_ms as f32 / 1000.0
        ),
        Event::Observation(observation) => println!("[{label}] {observation:?}"),
        Event::Sound { name, score } => println!("[{label}] heard {name} ({score:.3})"),
    }
}

fn downlinks(rig: RigId, event: &Event) -> Vec<Downlink> {
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

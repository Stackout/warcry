use crate::fault;
use anyhow::Result;
use std::process::{Command, Stdio};

pub fn say(text: &str) {
    let text = text.to_string();
    fault::spawn("text-to-speech", move || speak(&text));
}

fn quiet(mut cmd: Command) -> Command {
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    cmd
}

#[cfg(windows)]
fn speak(text: &str) -> Result<()> {
    use std::os::windows::process::CommandExt;
    let script = format!(
        "Add-Type -AssemblyName System.Speech; (New-Object System.Speech.Synthesis.SpeechSynthesizer).Speak('{}')",
        text.replace('\'', "''")
    );
    let mut cmd = quiet(Command::new("powershell"));
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(0x0800_0000);
    cmd.status().map(drop).map_err(Into::into)
}

#[cfg(target_os = "macos")]
fn speak(text: &str) -> Result<()> {
    quiet(Command::new("say"))
        .arg(text)
        .status()
        .map(drop)
        .map_err(Into::into)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn speak(text: &str) -> Result<()> {
    ["spd-say", "espeak-ng", "espeak"]
        .iter()
        .find_map(|prog| quiet(Command::new(prog)).arg(text).status().ok())
        .map(drop)
        .ok_or_else(|| {
            anyhow::anyhow!("no speech program found; install speech-dispatcher or espeak-ng")
        })
}

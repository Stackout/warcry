use super::WindowInfo;
use anyhow::{Context, Result, anyhow, ensure};
use std::process::{Command, Stdio};
use std::time::Duration;

const DEADLINE: Duration = Duration::from_secs(5);

fn osascript(script: &str) -> Result<String> {
    let child = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("could not run osascript")?;
    let pid = child.id();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("osascript".into())
        .spawn(move || drop(tx.send(child.wait_with_output())))?;
    let out = rx.recv_timeout(DEADLINE).map_err(|_| abandon(pid))??;
    ensure!(
        out.status.success(),
        "osascript failed: {} (grant Accessibility and Automation permission to your terminal)",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn abandon(pid: u32) -> anyhow::Error {
    drop(Command::new("kill").arg("-9").arg(pid.to_string()).status());
    anyhow!(
        "osascript did not answer within {}s; a window it asked about is not responding",
        DEADLINE.as_secs()
    )
}

const LIST: &str = r#"
set out to ""
tell application "System Events"
  with timeout of 3 seconds
    repeat with p in application processes
      try
        if background only of p is false then
          set pn to name of p
          set pid to unix id of p
          set wins to {pn}
          try
            if (count of windows of p) > 0 then set wins to name of every window of p
          end try
          repeat with w in wins
            set out to out & pid & tab & pn & tab & (w as text) & linefeed
          end repeat
        end if
      end try
    end repeat
  end timeout
end tell
return out
"#;

const ACTIVE: &str = r#"
tell application "System Events"
  set p to first process whose frontmost is true
  set t to name of p
  try
    set t to name of front window of p
  end try
  return (unix id of p as text) & tab & (name of p) & tab & t
end tell
"#;

fn parse(out: &str) -> Vec<WindowInfo> {
    out.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let pid = parts.next()?.trim().parse::<u32>().ok()?;
            Some((pid, parts.next()?.to_string(), parts.next()?.to_string()))
        })
        .scan((0u32, 0u64), |(last, idx), (pid, process, title)| {
            *idx = (pid == *last) as u64 * (*idx + 1);
            *last = pid;
            Some(WindowInfo {
                id: ((pid as u64) << 16) | *idx,
                pid,
                title,
                process,
                class: String::new(),
            })
        })
        .collect()
}

pub fn list() -> Result<Vec<WindowInfo>> {
    Ok(parse(&osascript(LIST)?))
}

pub fn front() -> Result<Option<WindowInfo>> {
    Ok(parse(osascript(ACTIVE)?.trim_end()).into_iter().next())
}

pub fn active() -> Result<Option<WindowInfo>> {
    let front = front()?;
    let known = |f: &WindowInfo| {
        list()
            .ok()?
            .into_iter()
            .find(|w| w.pid == f.pid && w.title == f.title)
    };
    Ok(front.map(|f| known(&f).unwrap_or(f)))
}

pub fn focus(w: &WindowInfo) -> Result<()> {
    let title = w.title.replace('\\', "\\\\").replace('"', "\\\"");
    osascript(&format!(
        r#"tell application "System Events"
  set p to first process whose unix id is {pid}
  set frontmost of p to true
  try
    perform action "AXRaise" of (first window of p whose name is "{title}")
  end try
end tell"#,
        pid = w.pid
    ))
    .map(drop)
}

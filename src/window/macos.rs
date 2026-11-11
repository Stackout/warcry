use super::WindowInfo;
use anyhow::{Result, ensure};
use std::process::Command;

fn osascript(script: &str) -> Result<String> {
    let out = Command::new("osascript").arg("-e").arg(script).output()?;
    ensure!(
        out.status.success(),
        "osascript failed: {} (grant Accessibility and Automation permission to your terminal)",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

const LIST: &str = r#"
set out to ""
tell application "System Events"
  repeat with p in (every process whose background only is false)
    set pn to name of p
    set pid to unix id of p
    set wins to {pn}
    try
      if (count of windows of p) > 0 then set wins to name of every window of p
    end try
    repeat with w in wins
      set out to out & pid & tab & pn & tab & (w as text) & linefeed
    end repeat
  end repeat
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

pub fn active() -> Result<Option<WindowInfo>> {
    let front = parse(osascript(ACTIVE)?.trim_end()).into_iter().next();
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

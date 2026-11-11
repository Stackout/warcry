use crate::profile::Target;
use anyhow::{Result, anyhow};
use std::fmt;
use std::time::Duration;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as imp;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as imp;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as imp;
#[cfg(windows)]
pub use windows::BackgroundSink;

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod imp {
    use super::WindowInfo;
    use anyhow::{Result, bail};
    pub fn list() -> Result<Vec<WindowInfo>> {
        bail!("window targeting is not supported on this OS")
    }
    pub fn active() -> Result<Option<WindowInfo>> {
        Ok(None)
    }
    pub fn focus(_: &WindowInfo) -> Result<()> {
        bail!("window targeting is not supported on this OS")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    pub id: u64,
    pub pid: u32,
    pub title: String,
    pub process: String,
    pub class: String,
}

impl fmt::Display for WindowInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} [{}, pid {}]", self.title, self.process, self.pid)
    }
}

pub fn list() -> Result<Vec<WindowInfo>> {
    imp::list()
}

pub fn active() -> Result<Option<WindowInfo>> {
    imp::active()
}

pub fn focus(w: &WindowInfo) -> Result<()> {
    imp::focus(w)?;
    let focused = (0..30).any(|_| {
        std::thread::sleep(Duration::from_millis(10));
        active().ok().flatten().is_some_and(|a| a.id == w.id)
    });
    (!focused).then(|| tracing::debug!("{w} did not report focus within 300ms"));
    Ok(())
}

fn contains(hay: &str, needle: &Option<String>) -> bool {
    needle
        .as_ref()
        .is_none_or(|n| hay.to_lowercase().contains(&n.to_lowercase()))
}

pub fn matches(w: &WindowInfo, t: &Target) -> bool {
    contains(&w.title, &t.title) && contains(&w.process, &t.process) && contains(&w.class, &t.class)
}

pub fn find(t: &Target) -> Result<WindowInfo> {
    let open = list()?;
    let exact = |w: &WindowInfo| {
        t.title
            .as_ref()
            .is_some_and(|x| x.eq_ignore_ascii_case(&w.title))
    };
    let picked = t
        .pid
        .and_then(|pid| open.iter().find(|w| w.pid == pid).cloned());
    picked
        .or_else(|| {
            open.iter()
                .filter(|w| matches(w, t))
                .min_by_key(|w| !exact(w))
                .cloned()
        })
        .ok_or_else(|| anyhow!("no window matches target {}", t.describe()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_exact_window_wins_over_a_title_match() {
        let make = |pid: u32, title: &str| WindowInfo {
            id: pid as u64,
            pid,
            title: title.into(),
            process: "Wow".into(),
            class: String::new(),
        };
        let open = [make(1, "World of Warcraft"), make(2, "World of Warcraft")];
        let target = Target {
            title: Some("World of Warcraft".into()),
            pid: Some(2),
            ..Default::default()
        };
        let picked = open
            .iter()
            .find(|w| target.pid.is_some_and(|pid| w.pid == pid))
            .unwrap();
        assert_eq!(picked.pid, 2, "the chosen window lost to a title match");
        assert!(open.iter().all(|w| matches(w, &target)));
    }

    #[test]
    fn target_matching() {
        let w = WindowInfo {
            id: 1,
            pid: 2,
            title: "Minecraft 1.21".into(),
            process: "javaw.exe".into(),
            class: "GLFW30".into(),
        };
        let t = |title: Option<&str>, process: Option<&str>| Target {
            title: title.map(Into::into),
            process: process.map(Into::into),
            ..Default::default()
        };
        assert!(matches(&w, &t(Some("minecraft"), None)));
        assert!(matches(&w, &t(Some("MINECRAFT"), Some("javaw"))));
        assert!(!matches(&w, &t(Some("minecraft"), Some("chrome"))));
    }
}

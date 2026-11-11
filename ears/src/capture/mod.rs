use anyhow::Result;
use crossbeam_channel::Sender;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as imp;

#[cfg(not(target_os = "macos"))]
mod imp {
    use anyhow::{Result, bail};
    use crossbeam_channel::Sender;

    pub struct Tap {
        pub rate: u32,
        pub channels: usize,
    }

    pub fn open(_pid: u32, _tx: Sender<Vec<f32>>) -> Result<Tap> {
        bail!("per-window audio capture is not implemented on this platform yet")
    }
}

pub use imp::Tap;

pub fn open(pid: u32, tx: Sender<Vec<f32>>) -> Result<Tap> {
    imp::open(pid, tx)
}

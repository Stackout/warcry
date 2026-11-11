#[cfg(feature = "download")]
use anyhow::{Context, ensure};
use anyhow::{Result, bail};
#[cfg(feature = "download")]
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const KNOWN: &[(&str, &str)] = &[
    ("tiny.en", "75 MB, fastest, least accurate"),
    ("base.en", "142 MB, recommended default"),
    ("small.en", "466 MB, more accurate, needs a fast CPU"),
    ("medium.en", "1.5 GB, most accurate English, slow on CPU"),
    ("tiny", "75 MB, multilingual"),
    ("base", "142 MB, multilingual"),
    ("small", "466 MB, multilingual"),
    ("large-v3-turbo", "1.6 GB, multilingual, best quality"),
];

pub fn dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("warcry")
        .join("models")
}

pub fn path_for(name: &str) -> PathBuf {
    dir().join(format!("ggml-{name}.bin"))
}

pub type Progress<'a> = &'a dyn Fn(u64, u64);

pub fn downloaded(spec: &str) -> bool {
    Path::new(spec).is_file() || path_for(spec).is_file()
}

pub fn resolve(spec: &str, download: Option<Progress>) -> Result<PathBuf> {
    let direct = Path::new(spec);
    match (direct.is_file(), path_for(spec).is_file(), download) {
        (true, _, _) => Ok(direct.to_path_buf()),
        (_, true, _) => Ok(path_for(spec)),
        (_, _, Some(progress)) => fetch(spec, progress),
        _ => bail!("model {spec:?} isn't downloaded; run `warcry model download {spec}`"),
    }
}

#[cfg(not(feature = "download"))]
pub fn fetch(name: &str, _: Progress) -> Result<PathBuf> {
    bail!(
        "this build can't download models; place ggml-{name}.bin in {}",
        dir().display()
    )
}

#[cfg(feature = "download")]
pub fn fetch(name: &str, progress: Progress) -> Result<PathBuf> {
    ensure!(
        KNOWN.iter().any(|(n, _)| *n == name),
        "unknown model {name:?}; choose one of: {}",
        KNOWN.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
    );
    let dest = path_for(name);
    let part = dest.with_extension("part");
    std::fs::create_dir_all(dir())?;
    let url = format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{name}.bin");
    tracing::info!("downloading {url}");
    let response = ureq::get(&url)
        .call()
        .with_context(|| format!("downloading {url}"))?;
    let total = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok()?.parse::<u64>().ok())
        .unwrap_or(0);
    let mut reader = Tracker {
        progress,
        inner: response
            .into_body()
            .into_with_config()
            .limit(u64::MAX)
            .reader(),
        read: 0,
        total,
        shown: 0,
    };
    progress(0, total);
    let mut file = std::fs::File::create(&part)?;
    std::io::copy(&mut reader, &mut file)?;
    file.flush()?;
    std::fs::rename(&part, &dest)?;
    tracing::info!("saved {}", dest.display());
    Ok(dest)
}

#[cfg(feature = "download")]
struct Tracker<'a, R> {
    progress: Progress<'a>,
    inner: R,
    read: u64,
    total: u64,
    shown: u64,
}

#[cfg(feature = "download")]
impl<R: Read> Read for Tracker<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.read += n as u64;
        let percent = self.read * 1000 / self.total.max(1);
        if percent > self.shown || n == 0 {
            self.shown = percent;
            (self.progress)(self.read, self.total);
        }
        Ok(n)
    }
}

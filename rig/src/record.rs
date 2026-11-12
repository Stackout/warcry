use crate::{Event, Tick};
use std::path::{Path, PathBuf};
use warcry_ears::detect::Observation;
use warcry_ears::{RATE, rms};

pub struct Recorder {
    dir: PathBuf,
    cast: Option<Cast>,
    casts: u32,
    run: u64,
}

struct Cast {
    samples: Vec<f32>,
    levels: Vec<f32>,
}

impl Recorder {
    pub fn new(dir: &Path) -> std::io::Result<Recorder> {
        std::fs::create_dir_all(dir)?;
        Ok(Recorder {
            dir: dir.to_path_buf(),
            cast: None,
            casts: 0,
            run: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_secs())
                .unwrap_or(0),
        })
    }

    pub fn observe(&mut self, pid: u32, tick: &Tick) {
        tick.events.iter().for_each(|event| self.mark(event));
        self.cast.iter_mut().for_each(|cast| {
            tick.frames.iter().for_each(|frame| {
                cast.samples.extend_from_slice(frame);
                cast.levels.push(rms(frame));
            })
        });
        tick.events
            .iter()
            .any(|event| matches!(event, Event::Observation(Observation::Disarmed { .. })))
            .then(|| self.finish(pid));
    }

    fn mark(&mut self, event: &Event) {
        matches!(event, Event::Observation(Observation::Armed { .. })).then(|| {
            self.cast = Some(Cast {
                samples: Vec::new(),
                levels: Vec::new(),
            })
        });
    }

    fn finish(&mut self, pid: u32) {
        let Some(cast) = self.cast.take() else {
            return;
        };
        self.casts += 1;
        let path = self
            .dir
            .join(format!("cast-{pid}-{}-{:03}.wav", self.run, self.casts));
        let written = write_wav(&path, &cast.samples);
        println!("  recorded {} {}", path.display(), summary(&cast.levels));
        drop(written.inspect_err(|e| println!("  could not write {}: {e}", path.display())));
    }
}

fn summary(levels: &[f32]) -> String {
    let settle = levels.len().min(50);
    let mut early: Vec<f32> = levels[..settle].to_vec();
    early.sort_by(|a, b| a.total_cmp(b));
    let floor = early.get(settle / 4).copied().unwrap_or(0.0);
    let peak = levels.iter().copied().fold(0.0f32, f32::max);
    let after = levels.get(settle..).unwrap_or(&[]);
    let peak_after = after.iter().copied().fold(0.0f32, f32::max);
    format!(
        "floor {floor:.5}  peak {peak:.5}  peak_after_settle {peak_after:.5}  ratio {:.1}x  frames {}",
        peak_after / floor.max(1e-9),
        levels.len()
    )
}

pub fn write_wav(path: &Path, samples: &[f32]) -> std::io::Result<()> {
    let pcm: Vec<u8> = samples
        .iter()
        .flat_map(|sample| ((sample.clamp(-1.0, 1.0) * 32_767.0) as i16).to_le_bytes())
        .collect();
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend(b"RIFF");
    out.extend(((36 + pcm.len()) as u32).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(RATE.to_le_bytes());
    out.extend((RATE * 2).to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend((pcm.len() as u32).to_le_bytes());
    out.extend(pcm);
    std::fs::write(path, out)
}

pub fn read_wav(path: &Path) -> std::io::Result<Vec<f32>> {
    let bytes = std::fs::read(path)?;
    let start = find_data(&bytes).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "no data chunk in wav")
    })?;
    Ok(bytes[start..]
        .chunks_exact(2)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]) as f32 / 32_768.0)
        .collect())
}

fn find_data(bytes: &[u8]) -> Option<usize> {
    (12..bytes.len().saturating_sub(8))
        .step_by(2)
        .find(|at| &bytes[*at..at + 4] == b"data")
        .map(|at| at + 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_runs_never_collide_on_a_filename() {
        let dir = std::env::temp_dir().join(format!("warcry-runs-{}", std::process::id()));
        let first = Recorder::new(&dir).unwrap();
        let mut second = Recorder::new(&dir).unwrap();
        second.run = first.run + 1;
        let name = |recorder: &Recorder, casts: u32| {
            format!("cast-{}-{}-{:03}.wav", 7, recorder.run, casts)
        };
        assert_ne!(name(&first, 1), name(&second, 1));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_wav_has_a_riff_header_and_two_bytes_per_sample() {
        let path = std::env::temp_dir().join(format!("warcry-wav-{}.wav", std::process::id()));
        write_wav(&path, &[0.0, 0.5, -0.5]).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(bytes.len(), 44 + 6);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_summary_reports_the_ratio_between_floor_and_peak() {
        let mut levels = vec![0.001; 50];
        levels.push(0.05);
        let text = summary(&levels);
        assert!(text.contains("ratio 50.0x"), "{text}");
    }
}

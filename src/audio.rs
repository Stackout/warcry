use crate::profile::Settings;
use anyhow::{Result, anyhow, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use crossbeam_channel::Sender;
use std::collections::VecDeque;

pub const RATE: u32 = 16_000;
const FRAME: usize = 320;
const PREROLL: usize = 4_800;
const ONSET_FRAMES: u32 = 3;

pub fn devices() -> Result<Vec<String>> {
    Ok(cpal::default_host()
        .input_devices()?
        .filter_map(|d| d.description().ok().map(|d| d.name().to_string()))
        .collect())
}

pub fn rms_of(samples: &[f32]) -> f32 {
    match samples.is_empty() {
        true => 0.0,
        false => (samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32).sqrt(),
    }
}

pub struct Mic {
    _stream: cpal::Stream,
    pub rate: u32,
    pub name: String,
}

pub fn open(name: Option<&str>, tx: Sender<Vec<f32>>) -> Result<Mic> {
    let host = cpal::default_host();
    let device = match name {
        None => host
            .default_input_device()
            .ok_or_else(|| anyhow!("no default microphone"))?,
        Some(n) => host
            .input_devices()?
            .find(|d| {
                d.description()
                    .is_ok_and(|x| x.name().to_lowercase().contains(&n.to_lowercase()))
            })
            .ok_or_else(|| anyhow!("no microphone matching {n:?}; see `warcry devices`"))?,
    };
    let config = device.default_input_config()?;
    let channels = config.channels() as usize;
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => build::<f32>(&device, config.config(), channels, tx)?,
        cpal::SampleFormat::I16 => build::<i16>(&device, config.config(), channels, tx)?,
        cpal::SampleFormat::U16 => build::<u16>(&device, config.config(), channels, tx)?,
        cpal::SampleFormat::I32 => build::<i32>(&device, config.config(), channels, tx)?,
        other => bail!("unsupported microphone sample format {other:?}"),
    };
    stream.play()?;
    let name = device
        .description()
        .map(|d| d.name().to_string())
        .unwrap_or_default();
    Ok(Mic {
        _stream: stream,
        rate: config.sample_rate(),
        name,
    })
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    channels: usize,
    tx: Sender<Vec<f32>>,
) -> Result<cpal::Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let on_data = move |data: &[T], _: &cpal::InputCallbackInfo| {
        let mono = data
            .chunks(channels)
            .map(|frame| frame.iter().map(|s| s.to_sample::<f32>()).sum::<f32>() / channels as f32)
            .collect();
        drop(tx.send(mono));
    };
    let on_error = |e| tracing::error!("microphone: {e}");
    Ok(device.build_input_stream(config, on_data, on_error, None)?)
}

pub fn read_wav(path: &std::path::Path) -> Result<Vec<f32>> {
    let reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    let channels = spec.channels as usize;
    let scale = 2f32.powi(spec.bits_per_sample as i32 - 1);
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.into_samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => reader
            .into_samples::<i32>()
            .map(|s| s.map(|v| v as f32 / scale))
            .collect::<Result<_, _>>()?,
    };
    let mono: Vec<f32> = samples
        .chunks(channels)
        .map(|f| f.iter().sum::<f32>() / channels as f32)
        .collect();
    Ok(Resampler::new(spec.sample_rate).process(&mono))
}

pub struct Resampler {
    step: f64,
    pos: f64,
    prev: f32,
    lowpass: f32,
    alpha: f32,
}

impl Resampler {
    pub fn new(rate: u32) -> Self {
        let alpha = 1.0 - (-2.0 * std::f32::consts::PI * 7_000.0 / rate as f32).exp();
        Self {
            step: rate as f64 / RATE as f64,
            pos: 0.0,
            prev: 0.0,
            lowpass: 0.0,
            alpha: alpha.min(1.0),
        }
    }

    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        let (alpha, start) = (self.alpha, self.lowpass);
        let filtered: Vec<f32> = input
            .iter()
            .scan(start, |lp, x| {
                *lp += alpha * (x - *lp);
                Some(*lp)
            })
            .collect();
        let buf: Vec<f32> = std::iter::once(self.prev).chain(filtered).collect();
        let limit = buf.len() as f64 - 1.0;
        let out: Vec<f32> = std::iter::successors(Some(self.pos), |p| Some(p + self.step))
            .take_while(|p| *p < limit)
            .map(|p| {
                let i = p as usize;
                buf[i] + (buf[i + 1] - buf[i]) * (p - i as f64) as f32
            })
            .collect();
        self.pos += out.len() as f64 * self.step - limit;
        self.prev = buf[buf.len() - 1];
        self.lowpass = self.prev;
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    Auto,
    Open,
    Closed,
}

pub struct Vad {
    min_threshold: f32,
    noise_multiplier: f32,
    silence_frames: u32,
    max_samples: usize,
    min_samples: usize,
    noise: f32,
    pending: Vec<f32>,
    preroll: VecDeque<f32>,
    speech: Vec<f32>,
    active: bool,
    loud: u32,
    quiet: u32,
}

impl Vad {
    pub fn new(s: &Settings) -> Self {
        let silence_frames = (s.silence_ms * RATE / 1000) as usize / FRAME;
        Self {
            min_threshold: s.vad_threshold,
            noise_multiplier: s.noise_multiplier,
            silence_frames: silence_frames as u32,
            max_samples: (s.max_utterance_ms * RATE / 1000) as usize,
            min_samples: (silence_frames + 10) * FRAME,
            noise: 0.0,
            pending: Vec::new(),
            preroll: VecDeque::new(),
            speech: Vec::new(),
            active: false,
            loud: 0,
            quiet: 0,
        }
    }

    pub fn push(&mut self, samples: &[f32], gate: Gate) -> Vec<Vec<f32>> {
        self.pending.extend_from_slice(samples);
        let whole = self.pending.len() / FRAME * FRAME;
        let frames: Vec<f32> = self.pending.drain(..whole).collect();
        let utterances: Vec<Vec<f32>> = frames
            .chunks(FRAME)
            .filter_map(|f| self.frame(f, gate))
            .collect();
        let released = (gate == Gate::Closed)
            .then(|| self.take())
            .filter(|u| u.len() >= FRAME * 10);
        utterances.into_iter().chain(released).collect()
    }

    fn frame(&mut self, f: &[f32], gate: Gate) -> Option<Vec<f32>> {
        match gate {
            Gate::Open => {
                self.speech.extend_from_slice(f);
                (self.speech.len() >= self.max_samples).then(|| self.take())
            }
            Gate::Closed => None,
            Gate::Auto => self.detect(f),
        }
    }

    fn detect(&mut self, f: &[f32]) -> Option<Vec<f32>> {
        let rms = (f.iter().map(|x| x * x).sum::<f32>() / f.len() as f32).sqrt();
        let loud = rms > self.min_threshold.max(self.noise * self.noise_multiplier);
        match (self.active, loud) {
            (false, false) => {
                self.noise = self.noise * 0.95 + rms * 0.05;
                self.loud = 0;
                self.remember(f);
                None
            }
            (false, true) => {
                self.noise = self.noise * 0.995 + rms * 0.005;
                self.loud += 1;
                self.remember(f);
                (self.loud >= ONSET_FRAMES).then(|| {
                    self.active = true;
                    self.speech = self.preroll.drain(..).collect();
                });
                None
            }
            (true, _) => {
                self.speech.extend_from_slice(f);
                self.quiet = (self.quiet + 1) * !loud as u32;
                (self.quiet >= self.silence_frames || self.speech.len() >= self.max_samples)
                    .then(|| self.take())
                    .filter(|u| u.len() >= self.min_samples)
            }
        }
    }

    fn remember(&mut self, f: &[f32]) {
        self.preroll.extend(f);
        let excess = self.preroll.len().saturating_sub(PREROLL);
        self.preroll.drain(..excess);
    }

    fn take(&mut self) -> Vec<f32> {
        self.active = false;
        self.loud = 0;
        self.quiet = 0;
        std::mem::take(&mut self.speech)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(len: usize, amp: f32) -> Vec<f32> {
        (0..len).map(|i| (i as f32 * 0.05).sin() * amp).collect()
    }

    #[test]
    fn resampler_ratio() {
        let mut r = Resampler::new(48_000);
        let total: usize = (0..100).map(|_| r.process(&tone(480, 0.5)).len()).sum();
        assert!((total as i64 - 16_000).abs() <= 2, "got {total}");
        let mut same = Resampler::new(16_000);
        let total: usize = (0..10).map(|_| same.process(&tone(1600, 0.5)).len()).sum();
        assert!((total as i64 - 16_000).abs() <= 2, "got {total}");
    }

    #[test]
    fn vad_segments_speech() {
        let mut vad = Vad::new(&Settings::default());
        let silence = vec![0.0; RATE as usize];
        let speech = tone(RATE as usize, 0.3);
        let out: Vec<Vec<f32>> = [&silence, &speech, &silence]
            .iter()
            .flat_map(|chunk| vad.push(chunk, Gate::Auto))
            .collect();
        assert_eq!(out.len(), 1);
        assert!(out[0].len() >= RATE as usize);
    }

    #[test]
    fn push_to_talk_gate() {
        let mut vad = Vad::new(&Settings::default());
        assert!(vad.push(&vec![0.0; 8000], Gate::Open).is_empty());
        let out = vad.push(&[], Gate::Closed);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].len(), 8000);
    }
}

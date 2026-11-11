use crate::{FRAME, RATE};
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::collections::VecDeque;
use std::sync::Arc;

pub const BANDS: usize = 24;
pub const WINDOW: usize = 512;

const LOW_HZ: f32 = 120.0;
const HIGH_HZ: f32 = 7_600.0;

pub struct Spectrogram {
    fft: Arc<dyn Fft<f32>>,
    taper: Vec<f32>,
    filters: Vec<Vec<f32>>,
    tail: VecDeque<f32>,
}

impl Spectrogram {
    pub fn new() -> Spectrogram {
        Spectrogram {
            fft: FftPlanner::new().plan_fft_forward(WINDOW),
            taper: hann(WINDOW),
            filters: filterbank(),
            tail: VecDeque::from(vec![0.0; WINDOW]),
        }
    }

    pub fn push(&mut self, frame: &[f32]) -> [f32; BANDS] {
        self.tail.extend(frame.iter().copied());
        let excess = self.tail.len().saturating_sub(WINDOW);
        self.tail.drain(..excess);
        let power = self.power();
        std::array::from_fn(|band| {
            let energy: f32 = self.filters[band]
                .iter()
                .zip(power.iter())
                .map(|(weight, bin)| weight * bin)
                .sum();
            (energy + 1e-10).ln()
        })
    }

    fn power(&self) -> Vec<f32> {
        let mut buffer: Vec<Complex<f32>> = self
            .tail
            .iter()
            .zip(self.taper.iter())
            .map(|(sample, weight)| Complex::new(sample * weight, 0.0))
            .collect();
        self.fft.process(&mut buffer);
        buffer[..WINDOW / 2]
            .iter()
            .map(|bin| bin.norm_sqr() / WINDOW as f32)
            .collect()
    }
}

impl Default for Spectrogram {
    fn default() -> Self {
        Spectrogram::new()
    }
}

pub fn analyze(samples: &[f32]) -> Vec<[f32; BANDS]> {
    let mut spectro = Spectrogram::new();
    samples
        .chunks(FRAME)
        .filter(|frame| frame.len() == FRAME)
        .map(|frame| spectro.push(frame))
        .collect()
}

pub fn flux(spectra: &[[f32; BANDS]]) -> Vec<f32> {
    spectra
        .windows(2)
        .map(|pair| {
            pair[1]
                .iter()
                .zip(pair[0].iter())
                .map(|(now, before)| (now - before).max(0.0))
                .sum()
        })
        .collect()
}

pub fn strongest_onset(spectra: &[[f32; BANDS]]) -> Option<usize> {
    let rising = flux(spectra);
    rising
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.total_cmp(b))
        .map(|(index, _)| index + 1)
}

fn hann(len: usize) -> Vec<f32> {
    (0..len)
        .map(|i| {
            let phase = 2.0 * std::f32::consts::PI * i as f32 / len as f32;
            0.5 - 0.5 * phase.cos()
        })
        .collect()
}

fn mel(hz: f32) -> f32 {
    2595.0 * (1.0 + hz / 700.0).log10()
}

fn unmel(mel: f32) -> f32 {
    700.0 * (10f32.powf(mel / 2595.0) - 1.0)
}

fn filterbank() -> Vec<Vec<f32>> {
    let bins = WINDOW / 2;
    let hz_per_bin = RATE as f32 / WINDOW as f32;
    let (low, high) = (mel(LOW_HZ), mel(HIGH_HZ));
    let edges: Vec<f32> = (0..BANDS + 2)
        .map(|i| unmel(low + (high - low) * i as f32 / (BANDS + 1) as f32) / hz_per_bin)
        .collect();
    (0..BANDS)
        .map(|band| {
            let (start, peak, end) = (edges[band], edges[band + 1], edges[band + 2]);
            (0..bins)
                .map(|bin| triangle(bin as f32, start, peak, end))
                .collect()
        })
        .collect()
}

fn triangle(at: f32, start: f32, peak: f32, end: f32) -> f32 {
    match at {
        _ if at <= start || at >= end => 0.0,
        _ if at <= peak => (at - start) / (peak - start).max(1e-6),
        _ => (end - at) / (end - peak).max(1e-6),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn tone(freq: f32, frames: usize) -> Vec<f32> {
        (0..frames * FRAME)
            .map(|i| (2.0 * PI * freq * i as f32 / RATE as f32).sin() * 0.4)
            .collect()
    }

    #[test]
    fn the_filterbank_covers_every_band() {
        let filters = filterbank();
        assert_eq!(filters.len(), BANDS);
        filters.iter().enumerate().for_each(|(band, weights)| {
            assert!(
                weights.iter().sum::<f32>() > 0.0,
                "band {band} has no weight"
            );
        });
    }

    #[test]
    fn a_low_tone_lights_a_low_band_and_a_high_tone_a_high_one() {
        let peak = |freq: f32| {
            let spectra = analyze(&tone(freq, 10));
            let last = spectra.last().copied().unwrap();
            last.iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.total_cmp(b))
                .map(|(band, _)| band)
                .unwrap()
        };
        let low = peak(300.0);
        let high = peak(5_000.0);
        assert!(low < 6, "300 Hz peaked at band {low}");
        assert!(high > 16, "5 kHz peaked at band {high}");
        assert!(low < high);
    }

    #[test]
    fn silence_is_quieter_in_every_band_than_a_tone() {
        let quiet = analyze(&vec![0.0; FRAME * 10]);
        let loud = analyze(&tone(1_000.0, 10));
        let quiet_sum: f32 = quiet.last().unwrap().iter().sum();
        let loud_sum: f32 = loud.last().unwrap().iter().sum();
        assert!(loud_sum > quiet_sum, "{loud_sum} should exceed {quiet_sum}");
    }

    #[test]
    fn flux_spikes_where_a_sound_begins() {
        let mut audio = vec![0.0; FRAME * 10];
        audio.extend(tone(2_000.0, 10));
        let spectra = analyze(&audio);
        let onset = strongest_onset(&spectra).unwrap();
        assert!((9..=12).contains(&onset), "onset found at frame {onset}");
    }

    #[test]
    fn an_empty_signal_has_no_onset() {
        assert_eq!(strongest_onset(&[]), None);
    }
}

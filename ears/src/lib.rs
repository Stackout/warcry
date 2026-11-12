pub mod capture;
pub mod detect;
pub mod library;
pub mod resample;
pub mod spectro;
pub mod tone;

pub const RATE: u32 = 16_000;

pub const FRAME: usize = 320;

pub fn rms(frame: &[f32]) -> f32 {
    match frame.is_empty() {
        true => 0.0,
        false => (frame.iter().map(|x| x * x).sum::<f32>() / frame.len() as f32).sqrt(),
    }
}

#[derive(Default)]
pub struct Framer {
    pending: Vec<f32>,
}

impl Framer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, samples: &[f32]) -> Vec<Vec<f32>> {
        self.pending.extend_from_slice(samples);
        let whole = self.pending.len() / FRAME * FRAME;
        self.pending
            .drain(..whole)
            .collect::<Vec<f32>>()
            .chunks(FRAME)
            .map(<[f32]>::to_vec)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rms_of_silence_is_zero() {
        assert_eq!(rms(&[0.0; FRAME]), 0.0);
        assert_eq!(rms(&[]), 0.0);
    }

    #[test]
    fn rms_of_full_scale_square_is_one() {
        let square: Vec<f32> = (0..FRAME).map(|i| 1.0 - 2.0 * (i % 2) as f32).collect();
        assert!((rms(&square) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn framer_emits_whole_frames_and_keeps_the_remainder() {
        let mut framer = Framer::new();
        assert_eq!(framer.push(&vec![0.0; FRAME - 1]).len(), 0);
        assert_eq!(framer.push(&[0.0; 2]).len(), 1);
        assert_eq!(framer.push(&vec![0.0; FRAME * 3]).len(), 3);
        assert_eq!(framer.push(&vec![0.0; FRAME - 1]).len(), 1);
    }

    #[test]
    fn framer_preserves_every_sample_in_order() {
        let mut framer = Framer::new();
        let input: Vec<f32> = (0..FRAME * 2).map(|i| i as f32).collect();
        let frames = framer.push(&input);
        let flat: Vec<f32> = frames.into_iter().flatten().collect();
        assert_eq!(flat, input);
    }
}

use crate::RATE;

pub struct Resampler {
    step: f64,
    pos: f64,
    prev: f32,
    lowpass: f32,
    alpha: f32,
}

impl Resampler {
    pub fn new(rate: u32) -> Self {
        let cutoff = (RATE as f32 / 2.0).min(rate as f32 / 2.0) * 0.9;
        let alpha = 1.0 - (-2.0 * std::f32::consts::PI * cutoff / rate as f32).exp();
        Resampler {
            step: rate as f64 / RATE as f64,
            pos: 0.0,
            prev: 0.0,
            lowpass: 0.0,
            alpha: alpha.min(1.0),
        }
    }

    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        match input.is_empty() {
            true => Vec::new(),
            false => self.interpolate(&self.filter(input)),
        }
    }

    fn filter(&self, input: &[f32]) -> Vec<f32> {
        let alpha = self.alpha;
        input
            .iter()
            .scan(self.lowpass, |level, x| {
                *level += alpha * (x - *level);
                Some(*level)
            })
            .collect()
    }

    fn interpolate(&mut self, filtered: &[f32]) -> Vec<f32> {
        let buf: Vec<f32> = std::iter::once(self.prev)
            .chain(filtered.iter().copied())
            .collect();
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

pub fn downmix(interleaved: &[f32], channels: usize) -> Vec<f32> {
    match channels <= 1 {
        true => interleaved.to_vec(),
        false => interleaved
            .chunks(channels)
            .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(len: usize, freq: f32, rate: u32) -> Vec<f32> {
        (0..len)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin() * 0.5)
            .collect()
    }

    #[test]
    fn forty_eight_kilohertz_lands_on_sixteen() {
        let mut r = Resampler::new(48_000);
        let total: usize = (0..100)
            .map(|_| r.process(&tone(480, 440.0, 48_000)).len())
            .sum();
        assert!((total as i64 - 16_000).abs() <= 2, "got {total}");
    }

    #[test]
    fn forty_four_one_lands_on_sixteen() {
        let mut r = Resampler::new(44_100);
        let total: usize = (0..100)
            .map(|_| r.process(&tone(441, 440.0, 44_100)).len())
            .sum();
        assert!((total as i64 - 16_000).abs() <= 3, "got {total}");
    }

    #[test]
    fn a_matching_rate_passes_through_sample_for_sample() {
        let mut r = Resampler::new(RATE);
        let total: usize = (0..10)
            .map(|_| r.process(&tone(1_600, 440.0, RATE)).len())
            .sum();
        assert!((total as i64 - 16_000).abs() <= 2, "got {total}");
    }

    #[test]
    fn empty_input_yields_nothing() {
        let mut r = Resampler::new(48_000);
        assert_eq!(r.process(&[]), Vec::<f32>::new());
    }

    #[test]
    fn a_beacon_tone_survives_the_trip_down_from_forty_eight() {
        let mut r = Resampler::new(48_000);
        let out: Vec<f32> = (0..50)
            .flat_map(|i| {
                let chunk: Vec<f32> = (0..480)
                    .map(|n| {
                        let t = (i * 480 + n) as f32 / 48_000.0;
                        (2.0 * std::f32::consts::PI * 1300.0 * t).sin() * 0.5
                    })
                    .collect();
                r.process(&chunk)
            })
            .collect();
        let peak = crate::tone::ToneBank::new(2)
            .push(&out[out.len() - crate::FRAME * 2..])
            .map(|mags| mags[0]);
        assert!(
            peak.is_some_and(|m| m > 0.3),
            "beacon lost in resampling: {peak:?}"
        );
    }

    #[test]
    fn downmix_averages_stereo_to_mono() {
        assert_eq!(downmix(&[1.0, 0.0, 0.5, 0.5], 2), vec![0.5, 0.5]);
        assert_eq!(downmix(&[1.0, 2.0], 1), vec![1.0, 2.0]);
    }
}

use crate::{FRAME, RATE, rms};

const FRAME_MS: u32 = (FRAME as u32 * 1000) / RATE;

#[derive(Clone, Copy, Debug)]
pub struct OnsetCfg {
    pub settle_ms: u32,
    pub ratio: f32,
    pub min_rms: f32,
    pub hot_frames: u32,
    pub floor_alpha: f32,
}

impl Default for OnsetCfg {
    fn default() -> Self {
        OnsetCfg {
            settle_ms: 1_000,
            ratio: 8.0,
            min_rms: 0.0015,
            hot_frames: 2,
            floor_alpha: 0.05,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub confidence: f32,
    pub rms: f32,
    pub floor: f32,
}

pub struct Onset {
    cfg: OnsetCfg,
    floor: f32,
    settling: Vec<f32>,
    settle_frames: u32,
    seen: u32,
    hot: u32,
}

impl Onset {
    pub fn new(cfg: OnsetCfg) -> Self {
        let settle_frames = cfg.settle_ms.div_ceil(FRAME_MS);
        Onset {
            cfg,
            floor: 0.0,
            settling: Vec::with_capacity(settle_frames as usize),
            settle_frames,
            seen: 0,
            hot: 0,
        }
    }

    pub fn rearm(&mut self) {
        self.floor = 0.0;
        self.settling.clear();
        self.seen = 0;
        self.hot = 0;
    }

    pub fn settling(&self) -> bool {
        self.seen < self.settle_frames
    }

    pub fn floor(&self) -> f32 {
        self.floor
    }

    pub fn push(&mut self, frame: &[f32]) -> Option<Hit> {
        let level = rms(frame);
        self.seen += 1;
        match self.settling() {
            true => {
                self.settling.push(level);
                None
            }
            false => {
                if !self.settling.is_empty() {
                    self.floor = low_percentile(&mut self.settling);
                    self.settling.clear();
                }
                self.fire(level)
            }
        }
    }

    fn fire(&mut self, level: f32) -> Option<Hit> {
        let threshold = self.cfg.min_rms.max(self.floor * self.cfg.ratio);
        match level > threshold {
            false => {
                self.floor += self.cfg.floor_alpha * (level - self.floor);
                self.hot = 0;
                None
            }
            true => {
                self.hot += 1;
                (self.hot == self.cfg.hot_frames).then(|| Hit {
                    confidence: confidence(level, threshold),
                    rms: level,
                    floor: self.floor,
                })
            }
        }
    }
}

fn confidence(level: f32, threshold: f32) -> f32 {
    match threshold > 0.0 {
        false => 1.0,
        true => (0.5 + 0.5 * (level / threshold - 1.0)).clamp(0.5, 1.0),
    }
}

fn low_percentile(values: &mut [f32]) -> f32 {
    match values.is_empty() {
        true => 0.0,
        false => {
            values.sort_by(|a, b| a.total_cmp(b));
            values[values.len() / 4]
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    CastStarted { seconds: u16 },
    CastEnded { caught: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Caught,
    Expired,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Observation {
    Armed { ends_in_ms: u32 },
    Bite { confidence: f32, into_cast_ms: u32 },
    Disarmed { outcome: Outcome },
}

pub struct ListenCfg {
    pub deaf_ms: u32,
}

impl Default for ListenCfg {
    fn default() -> Self {
        ListenCfg { deaf_ms: 600 }
    }
}

pub struct Listener {
    armed: Option<Arm>,
    frames: u64,
    deaf_frames: u64,
}

struct Arm {
    started: u64,
    expires: u64,
    fired: bool,
}

impl Listener {
    pub fn new(cfg: ListenCfg) -> Self {
        Listener {
            armed: None,
            frames: 0,
            deaf_frames: cfg.deaf_ms.div_ceil(FRAME_MS) as u64,
        }
    }

    pub fn is_armed(&self) -> bool {
        self.armed.is_some()
    }

    pub fn cue(&mut self, cue: Cue) -> Option<Observation> {
        match cue {
            Cue::CastStarted { seconds } => {
                let span = (seconds as u64 * 1000) / FRAME_MS as u64;
                self.armed = Some(Arm {
                    started: self.frames,
                    expires: self.frames + span,
                    fired: false,
                });
                Some(Observation::Armed {
                    ends_in_ms: seconds as u32 * 1000,
                })
            }
            Cue::CastEnded { caught } => self.armed.take().map(|_| Observation::Disarmed {
                outcome: match caught {
                    true => Outcome::Caught,
                    false => Outcome::Cancelled,
                },
            }),
        }
    }

    pub fn tick(&mut self) -> Option<Observation> {
        self.frames += 1;
        let expires = self.armed.as_ref()?.expires;
        (self.frames > expires).then(|| {
            self.armed = None;
            Observation::Disarmed {
                outcome: Outcome::Expired,
            }
        })
    }

    pub fn heard(&mut self, confidence: f32) -> Option<Observation> {
        let arm = self.armed.as_ref()?;
        let elapsed = self.frames.saturating_sub(arm.started);
        (elapsed >= self.deaf_frames && !arm.fired).then_some(())?;
        self.armed.as_mut()?.fired = true;
        Some(Observation::Bite {
            confidence,
            into_cast_ms: elapsed as u32 * FRAME_MS,
        })
    }
}

#[cfg(test)]
mod listener_tests {
    use super::*;

    fn armed() -> Listener {
        let mut listener = Listener::new(ListenCfg::default());
        listener.cue(Cue::CastStarted { seconds: 30 });
        (0..40).for_each(|_| {
            listener.tick();
        });
        listener
    }

    #[test]
    fn a_sound_outside_a_cast_is_not_a_bite() {
        let mut listener = Listener::new(ListenCfg::default());
        (0..100).for_each(|_| {
            listener.tick();
        });
        assert_eq!(listener.heard(0.9), None);
    }

    #[test]
    fn a_sound_inside_a_cast_is_a_bite() {
        let mut listener = armed();
        assert!(matches!(
            listener.heard(0.87),
            Some(Observation::Bite { confidence, .. }) if confidence == 0.87
        ));
    }

    #[test]
    fn only_the_first_sound_in_a_cast_is_a_bite() {
        let mut listener = armed();
        assert!(listener.heard(0.9).is_some());
        assert_eq!(listener.heard(0.9), None);
    }

    #[test]
    fn the_first_moments_of_a_cast_are_deaf() {
        let mut listener = Listener::new(ListenCfg::default());
        listener.cue(Cue::CastStarted { seconds: 30 });
        listener.tick();
        assert_eq!(listener.heard(0.99), None, "fired during the cast sound");
        (0..40).for_each(|_| {
            listener.tick();
        });
        assert!(listener.heard(0.99).is_some());
    }

    #[test]
    fn a_cast_expires_on_its_own() {
        let mut listener = Listener::new(ListenCfg::default());
        listener.cue(Cue::CastStarted { seconds: 1 });
        let ended: Vec<Observation> = (0..60).filter_map(|_| listener.tick()).collect();
        assert_eq!(
            ended,
            vec![Observation::Disarmed {
                outcome: Outcome::Expired
            }]
        );
        assert!(!listener.is_armed());
        assert_eq!(listener.heard(0.99), None);
    }

    #[test]
    fn catching_a_fish_disarms_the_window() {
        let mut listener = armed();
        assert_eq!(
            listener.cue(Cue::CastEnded { caught: true }),
            Some(Observation::Disarmed {
                outcome: Outcome::Caught
            })
        );
        assert_eq!(listener.heard(0.99), None);
    }

    #[test]
    fn a_stray_cast_end_is_harmless() {
        let mut listener = Listener::new(ListenCfg::default());
        assert_eq!(listener.cue(Cue::CastEnded { caught: false }), None);
    }

    #[test]
    fn arming_reports_the_window_length() {
        let mut listener = Listener::new(ListenCfg::default());
        assert_eq!(
            listener.cue(Cue::CastStarted { seconds: 30 }),
            Some(Observation::Armed { ends_in_ms: 30_000 })
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn burst(amp: f32) -> Vec<f32> {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        (0..FRAME)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                ((state >> 40) as f32 / 8_388_608.0 - 1.0) * amp
            })
            .collect()
    }

    fn quiet_room() -> Vec<f32> {
        burst(0.0008)
    }

    #[test]
    fn an_onset_needs_two_consecutive_loud_frames() {
        let mut onset = Onset::new(OnsetCfg::default());
        (0..60).for_each(|_| {
            onset.push(&quiet_room());
        });
        assert_eq!(onset.push(&burst(0.5)), None);
        assert!(onset.push(&burst(0.5)).is_some());
    }

    #[test]
    fn a_loud_channel_throughout_does_not_fire() {
        let mut onset = Onset::new(OnsetCfg::default());
        let fired: Vec<Hit> = (0..300).filter_map(|_| onset.push(&burst(0.3))).collect();
        assert_eq!(fired, vec![]);
    }

    #[test]
    fn digital_silence_does_not_fire_on_the_faintest_sample() {
        let mut onset = Onset::new(OnsetCfg::default());
        (0..60).for_each(|_| {
            onset.push(&vec![0.0; FRAME]);
        });
        let fired: Vec<Hit> = (0..10).filter_map(|_| onset.push(&burst(0.0005))).collect();
        assert_eq!(fired, vec![]);
    }

    #[test]
    fn rearming_forgets_the_old_channel() {
        let mut onset = Onset::new(OnsetCfg::default());
        (0..60).for_each(|_| {
            onset.push(&burst(0.3));
        });
        onset.rearm();
        assert!(onset.settling());
    }

    #[test]
    fn confidence_rises_with_margin_over_the_threshold() {
        assert_eq!(confidence(1.0, 1.0), 0.5);
        assert_eq!(confidence(1.5, 1.0), 0.75);
        assert_eq!(confidence(2.0, 1.0), 1.0);
        assert_eq!(confidence(9.0, 1.0), 1.0);
    }

    #[test]
    fn low_percentile_ignores_a_loud_quarter() {
        let mut values = [0.001, 0.001, 0.001, 0.001, 0.9, 0.9];
        assert_eq!(low_percentile(&mut values), 0.001);
    }

    #[test]
    fn a_frame_is_twenty_milliseconds() {
        assert_eq!(FRAME_MS, 20);
    }
}

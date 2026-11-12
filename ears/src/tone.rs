use crate::{FRAME, RATE};
use std::collections::VecDeque;
use std::f32::consts::PI;

pub const FREQS: [f32; 8] = [
    1300.0, 1700.0, 2150.0, 2550.0, 2950.0, 3350.0, 3750.0, 4150.0,
];

const IDENTITY_MARK: u8 = 7;

#[derive(Clone, Copy, Debug)]
pub struct ToneCfg {
    pub window_frames: usize,
    pub hold_frames: u32,
    pub dominance: f32,
    pub min_magnitude: f32,
    pub identity_window_frames: u32,
}

impl Default for ToneCfg {
    fn default() -> Self {
        ToneCfg {
            window_frames: 2,
            hold_frames: 4,
            dominance: 6.0,
            min_magnitude: 0.004,
            identity_window_frames: 75,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Beacon {
    CastStarted,
    CastEnded,
    Caught,
    BagsFull,
    LureExpiring,
    LureGone,
    Attention,
    Identity(u16),
}

fn beacon_for(symbol: u8) -> Option<Beacon> {
    match symbol {
        0 => Some(Beacon::CastStarted),
        1 => Some(Beacon::CastEnded),
        2 => Some(Beacon::Caught),
        3 => Some(Beacon::BagsFull),
        4 => Some(Beacon::LureExpiring),
        5 => Some(Beacon::LureGone),
        6 => Some(Beacon::Attention),
        _ => None,
    }
}

pub const IDENTITY_DIGITS: usize = 3;
pub const IDENTITY_SPACE: u16 = 1 << (3 * IDENTITY_DIGITS as u16);

pub fn identity_hash(name: &str) -> u16 {
    let folded = name
        .bytes()
        .map(|b| b.to_ascii_lowercase())
        .fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
    (folded % IDENTITY_SPACE as u32) as u16
}

pub struct ToneBank {
    window: usize,
    buf: VecDeque<f32>,
}

impl ToneBank {
    pub fn new(window_frames: usize) -> Self {
        let window = window_frames.max(1) * FRAME;
        ToneBank {
            window,
            buf: VecDeque::with_capacity(window),
        }
    }

    pub fn push(&mut self, frame: &[f32]) -> Option<[f32; FREQS.len()]> {
        self.buf.extend(frame.iter().copied());
        let excess = self.buf.len().saturating_sub(self.window);
        self.buf.drain(..excess);
        if self.buf.len() < self.window {
            return None;
        }
        let samples: Vec<f32> = self.buf.iter().copied().collect();
        Some(std::array::from_fn(|i| goertzel(&samples, FREQS[i])))
    }
}

fn goertzel(samples: &[f32], freq: f32) -> f32 {
    let n = samples.len() as f32;
    let bin = (n * freq / RATE as f32).round();
    let coeff = 2.0 * (2.0 * PI * bin / n).cos();
    let (s1, s2) = samples
        .iter()
        .fold((0.0f32, 0.0f32), |(s1, s2), &x| (x + coeff * s1 - s2, s1));
    ((s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0)).sqrt() / (n / 2.0)
}

pub struct Decoder {
    bank: ToneBank,
    cfg: ToneCfg,
    run: Option<Run>,
    identity: Option<Identity>,
}

struct Run {
    symbol: u8,
    frames: u32,
    emitted: bool,
}

struct Identity {
    digits: Vec<u8>,
    age: u32,
}

impl Decoder {
    pub fn new(cfg: ToneCfg) -> Self {
        Decoder {
            bank: ToneBank::new(cfg.window_frames),
            cfg,
            run: None,
            identity: None,
        }
    }

    pub fn push(&mut self, frame: &[f32]) -> Option<Beacon> {
        self.age_identity();
        let symbol = self.bank.push(frame).and_then(|mags| self.winner(&mags));
        let fresh = self.advance(symbol)?;
        self.accept(fresh)
    }

    fn winner(&self, mags: &[f32; FREQS.len()]) -> Option<u8> {
        let (index, &peak) = mags
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.total_cmp(b))?;
        let mut others: Vec<f32> = mags
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != index)
            .map(|(_, &m)| m)
            .collect();
        others.sort_by(|a, b| a.total_cmp(b));
        let median = others[others.len() / 2];
        let dominant = peak > self.cfg.min_magnitude && peak > median * self.cfg.dominance;
        dominant.then_some(index as u8)
    }

    fn advance(&mut self, symbol: Option<u8>) -> Option<u8> {
        let Some(symbol) = symbol else {
            self.run = None;
            return None;
        };
        let run = match self.run.take() {
            Some(run) if run.symbol == symbol => Run {
                frames: run.frames + 1,
                ..run
            },
            _ => Run {
                symbol,
                frames: 1,
                emitted: false,
            },
        };
        let ready = run.frames >= self.cfg.hold_frames && !run.emitted;
        self.run = Some(Run {
            emitted: run.emitted || ready,
            ..run
        });
        ready.then_some(symbol)
    }

    fn accept(&mut self, symbol: u8) -> Option<Beacon> {
        match self.identity.take() {
            Some(mut pending) => {
                pending.digits.push(symbol & 0x07);
                match pending.digits.len() == IDENTITY_DIGITS {
                    true => Some(Beacon::Identity(
                        pending
                            .digits
                            .iter()
                            .fold(0u16, |acc, digit| (acc << 3) | *digit as u16),
                    )),
                    false => {
                        self.identity = Some(pending);
                        None
                    }
                }
            }
            None if symbol == IDENTITY_MARK => {
                self.identity = Some(Identity {
                    digits: Vec::new(),
                    age: 0,
                });
                None
            }
            None => beacon_for(symbol),
        }
    }

    fn age_identity(&mut self) {
        self.identity = self.identity.take().and_then(|pending| {
            (pending.age < self.cfg.identity_window_frames).then_some(Identity {
                age: pending.age + 1,
                ..pending
            })
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freq: f32, ms: u32, amp: f32) -> Vec<Vec<f32>> {
        let total = (RATE * ms / 1000) as usize;
        (0..total)
            .map(|i| (2.0 * PI * freq * i as f32 / RATE as f32).sin() * amp)
            .collect::<Vec<f32>>()
            .chunks(FRAME)
            .map(<[f32]>::to_vec)
            .collect()
    }

    fn hiss(ms: u32, amp: f32) -> Vec<Vec<f32>> {
        let total = (RATE * ms / 1000) as usize;
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        (0..total)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                ((state >> 40) as f32 / 8_388_608.0 - 1.0) * amp
            })
            .collect::<Vec<f32>>()
            .chunks(FRAME)
            .map(<[f32]>::to_vec)
            .collect()
    }

    fn drain(decoder: &mut Decoder, frames: Vec<Vec<f32>>) -> Vec<Beacon> {
        frames.iter().filter_map(|f| decoder.push(f)).collect()
    }

    #[test]
    fn each_frequency_decodes_to_its_own_event() {
        let expected = [
            Beacon::CastStarted,
            Beacon::CastEnded,
            Beacon::Caught,
            Beacon::BagsFull,
            Beacon::LureExpiring,
            Beacon::LureGone,
            Beacon::Attention,
        ];
        for (index, want) in expected.iter().enumerate() {
            let mut decoder = Decoder::new(ToneCfg::default());
            let heard = drain(&mut decoder, tone(FREQS[index], 150, 0.3));
            assert_eq!(heard, vec![*want], "frequency {} misdecoded", FREQS[index]);
        }
    }

    #[test]
    fn a_tone_is_reported_once_however_long_it_is_held() {
        let mut decoder = Decoder::new(ToneCfg::default());
        let heard = drain(&mut decoder, tone(FREQS[3], 2_000, 0.3));
        assert_eq!(heard, vec![Beacon::BagsFull]);
    }

    #[test]
    fn two_tones_in_sequence_decode_separately() {
        let mut decoder = Decoder::new(ToneCfg::default());
        let mut frames = tone(FREQS[0], 150, 0.3);
        frames.extend(hiss(80, 0.0005));
        frames.extend(tone(FREQS[2], 150, 0.3));
        assert_eq!(
            drain(&mut decoder, frames),
            vec![Beacon::CastStarted, Beacon::Caught]
        );
    }

    #[test]
    fn broadband_noise_is_not_a_tone() {
        let mut decoder = Decoder::new(ToneCfg::default());
        assert_eq!(drain(&mut decoder, hiss(3_000, 0.5)), vec![]);
    }

    #[test]
    fn silence_is_not_a_tone() {
        let mut decoder = Decoder::new(ToneCfg::default());
        let quiet: Vec<Vec<f32>> = (0..100).map(|_| vec![0.0; FRAME]).collect();
        assert_eq!(drain(&mut decoder, quiet), vec![]);
    }

    #[test]
    fn a_very_quiet_tone_is_ignored() {
        let mut decoder = Decoder::new(ToneCfg::default());
        assert_eq!(drain(&mut decoder, tone(FREQS[1], 300, 0.0001)), vec![]);
    }

    #[test]
    fn a_brief_narrowband_blip_is_rejected() {
        let mut decoder = Decoder::new(ToneCfg::default());
        assert_eq!(drain(&mut decoder, tone(FREQS[5], 40, 0.4)), vec![]);
    }

    #[test]
    fn identity_takes_a_mark_and_three_digits() {
        let mut decoder = Decoder::new(ToneCfg::default());
        let mut frames = tone(FREQS[IDENTITY_MARK as usize], 150, 0.3);
        for digit in [5usize, 2, 7] {
            frames.extend(hiss(80, 0.0005));
            frames.extend(tone(FREQS[digit], 150, 0.3));
        }
        assert_eq!(drain(&mut decoder, frames), vec![Beacon::Identity(0o527)]);
    }

    #[test]
    fn identity_space_is_wide_enough_to_separate_a_stable_of_alts() {
        assert_eq!(IDENTITY_SPACE, 512);
        let stable = [
            "Death-Tailor",
            "Hawk-Strider",
            "Onyx-Warden",
            "Shadow-Bane",
            "Tempest-Warden",
            "Tulari-Stormgale",
            "Veil-Warden",
            "Vex-Warden",
            "Delin-Stormgale",
            "Lokari-Stormgale",
            "Shade-Warden",
            "Sky-Warden",
        ];
        let mut seen: Vec<u16> = stable.iter().map(|name| identity_hash(name)).collect();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(
            seen.len(),
            before,
            "two characters hashed to the same identity"
        );
    }

    #[test]
    fn a_truncated_identity_does_not_swallow_the_next_event() {
        let mut decoder = Decoder::new(ToneCfg::default());
        let mut frames = tone(FREQS[IDENTITY_MARK as usize], 150, 0.3);
        frames.extend(hiss(2_000, 0.0005));
        frames.extend(tone(FREQS[3], 150, 0.3));
        assert_eq!(drain(&mut decoder, frames), vec![Beacon::BagsFull]);
    }

    #[test]
    fn a_tone_survives_being_mixed_under_game_noise() {
        let mut decoder = Decoder::new(ToneCfg::default());
        let beacon = tone(FREQS[0], 200, 0.25);
        let noise = hiss(200, 0.05);
        let mixed: Vec<Vec<f32>> = beacon
            .iter()
            .zip(noise.iter())
            .map(|(a, b)| a.iter().zip(b).map(|(x, y)| x + y).collect())
            .collect();
        assert_eq!(drain(&mut decoder, mixed), vec![Beacon::CastStarted]);
    }

    #[test]
    fn every_beacon_frequency_lands_on_an_exact_analysis_bin() {
        let window = (ToneCfg::default().window_frames * FRAME) as f32;
        for freq in FREQS {
            let bin = window * freq / RATE as f32;
            assert_eq!(bin, bin.round(), "{freq} Hz is not an exact bin");
        }
    }

    #[test]
    fn goertzel_measures_a_full_scale_sine_as_unity() {
        let samples: Vec<f32> = (0..FRAME * 2)
            .map(|i| (2.0 * PI * FREQS[0] * i as f32 / RATE as f32).sin())
            .collect();
        assert!((goertzel(&samples, FREQS[0]) - 1.0).abs() < 0.02);
        assert!(goertzel(&samples, FREQS[4]) < 0.02);
    }

    #[test]
    fn identity_hash_is_stable_and_case_insensitive() {
        assert_eq!(identity_hash("Thrallfisher"), identity_hash("thrallfisher"));
        assert!(identity_hash("Thrallfisher") < IDENTITY_SPACE);
        assert_ne!(identity_hash("Thrallfisher"), identity_hash("Bankalt"));
    }

    #[test]
    fn identity_hash_folds_case_the_way_lua_does() {
        assert_eq!(identity_hash("Bane"), identity_hash("BANE"));
        assert_eq!(identity_hash("Bäne"), identity_hash("bäne"));
        assert_ne!(identity_hash("Bäne"), identity_hash("BÄNE"));
    }

    #[test]
    fn identity_hash_matches_a_hand_worked_lua_result() {
        let lua = |name: &str| {
            name.bytes().fold(0u64, |acc, b| {
                (acc * 31 + b.to_ascii_lowercase() as u64) % 4_294_967_296
            }) % IDENTITY_SPACE as u64
        };
        for name in ["Death-Tailor", "Shadow-Bäne", "Tulari-Stormgale", "Vex"] {
            assert_eq!(identity_hash(name) as u64, lua(name), "{name}");
        }
    }
}

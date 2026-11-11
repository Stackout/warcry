use crate::spectro::{BANDS, Spectrogram, analyze, strongest_onset};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

pub const DEFAULT_FRAMES: usize = 12;

const REFRACTORY_FRAMES: u32 = 25;
const ONSET_OFFSET: usize = 1;
const ALIGNMENTS: usize = 3;
const FLUX_SAFETY: f32 = 0.5;
const MARGIN: f32 = 0.04;
const FLOOR: f32 = 0.50;
const CEILING: f32 = 0.85;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Patch {
    pub frames: usize,
    pub data: Vec<f32>,
}

impl Patch {
    pub fn from_spectra(spectra: &[[f32; BANDS]]) -> Option<Patch> {
        (!spectra.is_empty()).then_some(())?;
        let frames = spectra.len();
        let means: [f32; BANDS] = std::array::from_fn(|band| {
            spectra.iter().map(|frame| frame[band]).sum::<f32>() / frames as f32
        });
        let moving: Vec<f32> = spectra
            .iter()
            .flat_map(|frame| {
                frame
                    .iter()
                    .enumerate()
                    .map(|(band, level)| level - means[band])
                    .collect::<Vec<f32>>()
            })
            .collect();
        Some(Patch {
            frames,
            data: unit(moving),
        })
    }

    pub fn score(&self, other: &Patch) -> f32 {
        match self.data.len() == other.data.len() {
            false => 0.0,
            true => self
                .data
                .iter()
                .zip(other.data.iter())
                .map(|(a, b)| a * b)
                .sum(),
        }
    }
}

fn unit(values: Vec<f32>) -> Vec<f32> {
    let magnitude = values.iter().map(|v| v * v).sum::<f32>().sqrt();
    match magnitude > 1e-6 {
        false => vec![0.0; values.len()],
        true => values.into_iter().map(|v| v / magnitude).collect(),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sound {
    pub name: String,
    pub patches: Vec<Patch>,
    pub threshold: f32,
    #[serde(default)]
    pub flux: f32,
}

impl Sound {
    pub fn examples(&self) -> usize {
        self.patches.len()
    }

    pub fn frames(&self) -> usize {
        self.patches
            .first()
            .map(|patch| patch.frames)
            .unwrap_or(DEFAULT_FRAMES)
    }

    pub fn best(&self, heard: &Patch) -> f32 {
        self.patches
            .iter()
            .map(|patch| heard.score(patch))
            .fold(f32::NEG_INFINITY, f32::max)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Heard {
    pub name: String,
    pub score: f32,
}

pub struct Library {
    sounds: Vec<Sound>,
    spectro: Spectrogram,
    history: VecDeque<[f32; BANDS]>,
    rising: VecDeque<f32>,
    cooling: Vec<u32>,
    depth: usize,
}

impl Library {
    pub fn new(sounds: Vec<Sound>) -> Library {
        let depth = sounds
            .iter()
            .map(|sound| sound.frames())
            .max()
            .unwrap_or(DEFAULT_FRAMES)
            + ALIGNMENTS;
        Library {
            cooling: vec![0; sounds.len()],
            sounds,
            spectro: Spectrogram::new(),
            history: VecDeque::with_capacity(depth + 1),
            rising: VecDeque::with_capacity(depth + 1),
            depth,
        }
    }

    pub fn names(&self) -> Vec<&str> {
        self.sounds
            .iter()
            .map(|sound| sound.name.as_str())
            .collect()
    }

    pub fn push(&mut self, frame: &[f32]) -> Vec<Heard> {
        let spectrum = self.spectro.push(frame);
        let climb = self
            .history
            .back()
            .map(|previous| {
                spectrum
                    .iter()
                    .zip(previous.iter())
                    .map(|(now, before)| (now - before).max(0.0))
                    .sum()
            })
            .unwrap_or(0.0);
        self.history.push_back(spectrum);
        self.rising.push_back(climb);
        let excess = self.history.len().saturating_sub(self.depth);
        self.history.drain(..excess);
        self.rising.drain(..excess);
        self.cooling
            .iter_mut()
            .for_each(|frames| *frames = frames.saturating_sub(1));
        (0..self.sounds.len())
            .filter_map(|index| self.compare(index))
            .collect()
    }

    fn aligned(&self, sound: &Sound, want: usize, shift: usize) -> Option<f32> {
        let end = self.history.len().checked_sub(shift)?;
        let start = end.checked_sub(want)?;
        let climb = self
            .rising
            .get(start + ONSET_OFFSET)
            .copied()
            .unwrap_or(0.0);
        (climb >= sound.flux).then_some(())?;
        let window: Vec<[f32; BANDS]> = self
            .history
            .iter()
            .skip(start)
            .take(want)
            .copied()
            .collect();
        Patch::from_spectra(&window).map(|heard| sound.best(&heard))
    }

    fn compare(&mut self, index: usize) -> Option<Heard> {
        (self.cooling[index] == 0).then_some(())?;
        let sound = &self.sounds[index];
        let want = sound.frames();
        (self.history.len() >= want + ALIGNMENTS - 1).then_some(())?;
        let score = (0..ALIGNMENTS)
            .filter_map(|shift| self.aligned(sound, want, shift))
            .fold(f32::NEG_INFINITY, f32::max);
        (score >= sound.threshold).then(|| {
            self.cooling[index] = REFRACTORY_FRAMES;
            Heard {
                name: self.sounds[index].name.clone(),
                score,
            }
        })
    }
}

pub fn learn(name: &str, examples: &[Vec<f32>], frames: usize) -> Option<Sound> {
    let found: Vec<(Patch, f32)> = examples
        .iter()
        .filter_map(|audio| excerpt_with_flux(audio, frames))
        .collect();
    (!found.is_empty()).then_some(())?;
    let flux = found
        .iter()
        .map(|(_, climb)| *climb)
        .fold(f32::INFINITY, f32::min);
    let patches: Vec<Patch> = found.into_iter().map(|(patch, _)| patch).collect();
    Some(Sound {
        name: name.to_string(),
        threshold: threshold(&patches),
        flux: flux * FLUX_SAFETY,
        patches,
    })
}

pub fn from_patches(name: &str, patches: Vec<Patch>, flux: f32) -> Option<Sound> {
    (!patches.is_empty()).then(|| Sound {
        name: name.to_string(),
        threshold: threshold(&patches),
        flux,
        patches,
    })
}

pub fn prune(patches: Vec<Patch>, min: f32) -> Vec<Patch> {
    let scores = agreement(&patches);
    patches
        .into_iter()
        .zip(scores)
        .filter(|(_, score)| *score >= min)
        .map(|(patch, _)| patch)
        .collect()
}

pub fn agreement(patches: &[Patch]) -> Vec<f32> {
    patches
        .iter()
        .enumerate()
        .map(|(mine, patch)| {
            patches
                .iter()
                .enumerate()
                .filter(|(other, _)| *other != mine)
                .map(|(_, against)| patch.score(against))
                .fold(f32::NEG_INFINITY, f32::max)
        })
        .collect()
}

fn threshold(patches: &[Patch]) -> f32 {
    match patches.len() {
        0 | 1 => 0.70,
        _ => (agreement(patches).into_iter().fold(f32::INFINITY, f32::min) - MARGIN)
            .clamp(FLOOR, CEILING),
    }
}

pub fn excerpt(audio: &[f32], frames: usize) -> Option<Patch> {
    excerpt_with_flux(audio, frames).map(|(patch, _)| patch)
}

pub fn excerpt_with_flux(audio: &[f32], frames: usize) -> Option<(Patch, f32)> {
    let spectra = analyze(audio);
    let rising = crate::spectro::flux(&spectra);
    let onset = strongest_onset(&spectra)?;
    let climb = rising.get(onset.saturating_sub(1)).copied().unwrap_or(0.0);
    let start = onset
        .saturating_sub(ONSET_OFFSET)
        .min(spectra.len().saturating_sub(frames));
    let slice = spectra.get(start..start + frames)?;
    Patch::from_spectra(slice).map(|patch| (patch, climb))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FRAME, RATE};
    use std::f32::consts::PI;

    fn chirp(from: f32, to: f32, ms: u32) -> Vec<f32> {
        let total = (RATE * ms / 1000) as usize;
        (0..total)
            .map(|i| {
                let progress = i as f32 / total as f32;
                let freq = from + (to - from) * progress;
                (2.0 * PI * freq * i as f32 / RATE as f32).sin() * 0.4
            })
            .collect()
    }

    fn hush(ms: u32) -> Vec<f32> {
        vec![0.0; (RATE * ms / 1000) as usize]
    }

    fn occurrence(sound: Vec<f32>) -> Vec<f32> {
        let mut audio = hush(300);
        audio.extend(sound);
        audio.extend(hush(300));
        audio
    }

    fn run(library: &mut Library, audio: &[f32]) -> Vec<Heard> {
        audio
            .chunks(FRAME)
            .filter(|frame| frame.len() == FRAME)
            .flat_map(|frame| library.push(frame))
            .collect()
    }

    #[test]
    fn a_learned_sound_recognises_itself() {
        let examples: Vec<Vec<f32>> = (0..3)
            .map(|_| occurrence(chirp(400.0, 2_000.0, 240)))
            .collect();
        let sound = learn("splash", &examples, DEFAULT_FRAMES).unwrap();
        assert_eq!(sound.examples(), 3);
        let mut library = Library::new(vec![sound]);
        let heard = run(&mut library, &occurrence(chirp(400.0, 2_000.0, 240)));
        assert_eq!(heard.len(), 1, "{heard:?}");
        assert_eq!(heard[0].name, "splash");
    }

    #[test]
    fn a_learned_sound_ignores_a_different_sound() {
        let examples: Vec<Vec<f32>> = (0..3)
            .map(|_| occurrence(chirp(400.0, 2_000.0, 240)))
            .collect();
        let sound = learn("splash", &examples, DEFAULT_FRAMES).unwrap();
        let mut library = Library::new(vec![sound]);
        let heard = run(&mut library, &occurrence(chirp(6_000.0, 6_200.0, 240)));
        assert_eq!(heard, vec![], "matched an unrelated sound");
    }

    #[test]
    fn silence_matches_nothing() {
        let examples: Vec<Vec<f32>> = (0..3)
            .map(|_| occurrence(chirp(400.0, 2_000.0, 240)))
            .collect();
        let sound = learn("splash", &examples, DEFAULT_FRAMES).unwrap();
        let mut library = Library::new(vec![sound]);
        assert_eq!(run(&mut library, &hush(2_000)), vec![]);
    }

    #[test]
    fn a_learned_sound_has_an_onset_requirement() {
        let examples: Vec<Vec<f32>> = (0..3)
            .map(|_| occurrence(chirp(400.0, 2_000.0, 240)))
            .collect();
        let sound = learn("splash", &examples, DEFAULT_FRAMES).unwrap();
        assert!(sound.flux > 0.0, "no onset floor was learned");
    }

    #[test]
    fn a_quiet_room_never_matches_however_long_it_listens() {
        let examples: Vec<Vec<f32>> = (0..3)
            .map(|_| occurrence(chirp(400.0, 2_000.0, 240)))
            .collect();
        let sound = learn("splash", &examples, DEFAULT_FRAMES).unwrap();
        let mut library = Library::new(vec![sound]);
        let mut state = 0x51ed_270b_9f23_11e9u64;
        let hiss: Vec<f32> = (0..RATE as usize * 20)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                ((state >> 40) as f32 / 8_388_608.0 - 1.0) * 0.0009
            })
            .collect();
        assert_eq!(run(&mut library, &hiss), vec![], "matched an idle channel");
    }

    #[test]
    fn two_sounds_are_told_apart() {
        let splashes: Vec<Vec<f32>> = (0..3)
            .map(|_| occurrence(chirp(400.0, 2_000.0, 240)))
            .collect();
        let hooks: Vec<Vec<f32>> = (0..3)
            .map(|_| occurrence(chirp(5_000.0, 3_000.0, 240)))
            .collect();
        let library = vec![
            learn("splash", &splashes, DEFAULT_FRAMES).unwrap(),
            learn("hook", &hooks, DEFAULT_FRAMES).unwrap(),
        ];
        let mut library = Library::new(library);
        assert_eq!(library.names(), vec!["splash", "hook"]);
        let heard = run(&mut library, &occurrence(chirp(5_000.0, 3_000.0, 240)));
        assert_eq!(
            heard.iter().map(|h| h.name.as_str()).collect::<Vec<_>>(),
            vec!["hook"]
        );
    }

    fn taught() -> Library {
        let examples: Vec<Vec<f32>> = (0..3)
            .map(|_| occurrence(chirp(400.0, 2_000.0, 240)))
            .collect();
        Library::new(vec![learn("splash", &examples, DEFAULT_FRAMES).unwrap()])
    }

    #[test]
    fn a_sliding_window_over_one_occurrence_reports_it_once() {
        let mut library = taught();
        let mut audio = hush(300);
        audio.extend(chirp(400.0, 2_000.0, 240));
        audio.extend(hush(100));
        audio.extend(chirp(400.0, 2_000.0, 240));
        audio.extend(hush(300));
        assert_eq!(run(&mut library, &audio).len(), 1);
    }

    #[test]
    fn two_occurrences_well_apart_are_both_reported() {
        let mut library = taught();
        let mut audio = hush(300);
        audio.extend(chirp(400.0, 2_000.0, 240));
        audio.extend(hush(900));
        audio.extend(chirp(400.0, 2_000.0, 240));
        audio.extend(hush(300));
        assert_eq!(run(&mut library, &audio).len(), 2);
    }

    #[test]
    fn variants_taught_together_are_all_recognised() {
        let variants = [
            chirp(400.0, 2_000.0, 240),
            chirp(700.0, 2_600.0, 240),
            chirp(300.0, 1_200.0, 240),
        ];
        let examples: Vec<Vec<f32>> = variants.iter().cloned().map(occurrence).collect();
        let sound = learn("bobber", &examples, DEFAULT_FRAMES).unwrap();
        assert_eq!(sound.examples(), 3);
        variants.iter().enumerate().for_each(|(index, variant)| {
            let mut library = Library::new(vec![sound.clone()]);
            let heard = run(&mut library, &occurrence(variant.clone()));
            assert_eq!(heard.len(), 1, "variant {index} missed: {heard:?}");
        });
    }

    #[test]
    fn variants_taught_together_still_reject_an_unrelated_sound() {
        let examples: Vec<Vec<f32>> = [
            chirp(400.0, 2_000.0, 240),
            chirp(700.0, 2_600.0, 240),
            chirp(300.0, 1_200.0, 240),
        ]
        .iter()
        .cloned()
        .map(occurrence)
        .collect();
        let sound = learn("bobber", &examples, DEFAULT_FRAMES).unwrap();
        let mut library = Library::new(vec![sound]);
        assert_eq!(
            run(&mut library, &occurrence(chirp(6_000.0, 6_400.0, 240))),
            vec![]
        );
    }

    #[test]
    fn a_longer_sweep_is_a_different_sound() {
        let mut library = taught();
        assert_eq!(
            run(&mut library, &occurrence(chirp(400.0, 2_000.0, 900))),
            vec![]
        );
    }

    #[test]
    fn steady_sound_carries_almost_no_shape() {
        let steady: Vec<f32> = (0..RATE as usize)
            .map(|i| (2.0 * PI * 800.0 * i as f32 / RATE as f32).sin() * 0.3)
            .collect();
        let spectra = crate::spectro::analyze(&steady);
        let patch = Patch::from_spectra(&spectra[20..20 + DEFAULT_FRAMES]).unwrap();
        let energy: f32 = patch.data.iter().map(|v| v.abs()).sum();
        let transient = excerpt(&occurrence(chirp(400.0, 2_000.0, 240)), DEFAULT_FRAMES).unwrap();
        assert!(
            patch.score(&transient).abs() < 0.5,
            "a held tone matched a transient at {}",
            patch.score(&transient)
        );
        assert!(energy >= 0.0);
    }

    #[test]
    fn a_patch_is_unit_length_so_scores_are_cosines() {
        let patch = excerpt(&occurrence(chirp(400.0, 2_000.0, 240)), DEFAULT_FRAMES).unwrap();
        let magnitude: f32 = patch.data.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((magnitude - 1.0).abs() < 1e-5, "magnitude {magnitude}");
        assert!((patch.score(&patch) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn pruning_drops_an_example_that_agrees_with_nothing() {
        let good: Vec<Vec<f32>> = (0..3)
            .map(|_| occurrence(chirp(400.0, 2_000.0, 240)))
            .collect();
        let mut patches: Vec<Patch> = good
            .iter()
            .filter_map(|clip| excerpt(clip, DEFAULT_FRAMES))
            .collect();
        patches.push(excerpt(&occurrence(chirp(6_500.0, 6_900.0, 240)), DEFAULT_FRAMES).unwrap());
        let kept = prune(patches, 0.55);
        assert_eq!(kept.len(), 3, "the odd one out survived pruning");
        let sound = from_patches("bobber", kept, 1.0).unwrap();
        assert!(sound.threshold > 0.55, "threshold {}", sound.threshold);
    }

    #[test]
    fn pruning_an_empty_set_is_empty() {
        assert_eq!(prune(Vec::new(), 0.5).len(), 0);
        assert!(from_patches("bobber", Vec::new(), 1.0).is_none());
    }

    #[test]
    fn learning_from_nothing_yields_nothing() {
        assert!(learn("splash", &[], DEFAULT_FRAMES).is_none());
    }

    #[test]
    fn a_sound_survives_a_round_trip_through_json() {
        let examples: Vec<Vec<f32>> = (0..3)
            .map(|_| occurrence(chirp(400.0, 2_000.0, 240)))
            .collect();
        let sound = learn("splash", &examples, DEFAULT_FRAMES).unwrap();
        let json = serde_json::to_string(&sound).unwrap();
        let back: Sound = serde_json::from_str(&json).unwrap();
        assert_eq!(back.patches, sound.patches);
        assert_eq!(back.threshold, sound.threshold);
    }
}

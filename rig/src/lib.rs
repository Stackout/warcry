pub mod companion;
pub mod deck;
pub mod find;
pub mod hub;
pub mod names;
pub mod record;

use anyhow::Result;
use crossbeam_channel::{Sender, unbounded};
use warcry_ears::Framer;
use warcry_ears::capture::{self, Tap};
use warcry_ears::detect::{Cue, ListenCfg, Listener, Observation};
use warcry_ears::library::{Library, Sound};
use warcry_ears::resample::Resampler;
use warcry_ears::tone::{Beacon, Decoder, ToneCfg};

pub const CAST_SECONDS: u16 = 30;

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Beacon(Beacon),
    Observation(Observation),
    Sound { name: String, score: f32 },
}

pub struct Tick {
    pub events: Vec<Event>,
    pub frames: Vec<Vec<f32>>,
}

pub struct Pipeline {
    resampler: Resampler,
    framer: Framer,
    decoder: Decoder,
    listener: Listener,
    library: Library,
    bite: String,
}

impl Pipeline {
    pub fn new(rate: u32) -> Self {
        Pipeline::with_sounds(rate, Vec::new(), "bobber")
    }

    pub fn with_sounds(rate: u32, sounds: Vec<Sound>, bite: &str) -> Self {
        Pipeline {
            resampler: Resampler::new(rate),
            framer: Framer::new(),
            decoder: Decoder::new(ToneCfg::default()),
            listener: Listener::new(ListenCfg::default()),
            library: Library::new(sounds),
            bite: bite.to_string(),
        }
    }

    pub fn push(&mut self, samples: &[f32]) -> Vec<Event> {
        self.advance(samples).events
    }

    pub fn advance(&mut self, samples: &[f32]) -> Tick {
        let resampled = self.resampler.process(samples);
        let frames = self.framer.push(&resampled);
        let events = frames.iter().flat_map(|frame| self.frame(frame)).collect();
        Tick { events, frames }
    }

    fn frame(&mut self, frame: &[f32]) -> Vec<Event> {
        let beacon = self.decoder.push(frame);
        let cued = beacon.and_then(|heard| self.cue(heard));
        let expired = self.listener.tick();
        let heard = self.library.push(frame);
        let named: Vec<Event> = heard
            .iter()
            .map(|sound| Event::Sound {
                name: sound.name.clone(),
                score: sound.score,
            })
            .collect();
        let bite = self.bite.clone();
        let bites: Vec<Event> = heard
            .iter()
            .filter(|sound| sound.name == bite)
            .filter_map(|sound| self.listener.heard(sound.score))
            .map(Event::Observation)
            .collect();
        beacon
            .map(Event::Beacon)
            .into_iter()
            .chain(cued.map(Event::Observation))
            .chain(expired.map(Event::Observation))
            .chain(named)
            .chain(bites)
            .collect()
    }

    fn cue(&mut self, beacon: Beacon) -> Option<Observation> {
        match beacon {
            Beacon::CastStarted => self.listener.cue(Cue::CastStarted {
                seconds: CAST_SECONDS,
            }),
            Beacon::Caught => self.listener.cue(Cue::CastEnded { caught: true }),
            Beacon::CastEnded => self.listener.cue(Cue::CastEnded { caught: false }),
            _ => None,
        }
    }
}

pub fn attach(
    pid: u32,
    events: Sender<(u32, Event)>,
    sounds: Vec<Sound>,
    bite: &str,
    recorder: Option<record::Recorder>,
) -> Result<Tap> {
    let (audio_tx, audio) = unbounded();
    let tap = capture::open(pid, audio_tx)?;
    let rate = tap.rate;
    let bite = bite.to_string();
    std::thread::Builder::new()
        .name(format!("rig-{pid}"))
        .spawn(move || {
            let mut pipeline = Pipeline::with_sounds(rate, sounds, &bite);
            let mut recorder = recorder;
            while let Ok(samples) = audio.recv() {
                let tick = pipeline.advance(&samples);
                recorder
                    .iter_mut()
                    .for_each(|recorder| recorder.observe(pid, &tick));
                let sent = tick
                    .events
                    .into_iter()
                    .try_for_each(|event| events.send((pid, event)));
                if sent.is_err() {
                    return;
                }
            }
        })?;
    Ok(tap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;
    use warcry_ears::library::{DEFAULT_FRAMES, learn};
    use warcry_ears::tone::FREQS;

    const DEVICE_RATE: u32 = 48_000;

    fn chirp(from: f32, to: f32, ms: u32) -> Vec<f32> {
        let total = (DEVICE_RATE * ms / 1000) as usize;
        (0..total)
            .map(|i| {
                let progress = i as f32 / total as f32;
                let freq = from + (to - from) * progress;
                (2.0 * PI * freq * i as f32 / DEVICE_RATE as f32).sin() * 0.3
            })
            .collect()
    }

    fn tone(freq: f32, ms: u32) -> Vec<f32> {
        (0..(DEVICE_RATE * ms / 1000) as usize)
            .map(|i| (2.0 * PI * freq * i as f32 / DEVICE_RATE as f32).sin() * 0.3)
            .collect()
    }

    fn hush(ms: u32) -> Vec<f32> {
        vec![0.0; (DEVICE_RATE * ms / 1000) as usize]
    }

    fn splash() -> Vec<f32> {
        chirp(500.0, 2_400.0, 240)
    }

    fn taught() -> Vec<Sound> {
        let examples: Vec<Vec<f32>> = (0..3)
            .map(|_| {
                let mut clip = hush(200);
                clip.extend(splash());
                clip.extend(hush(200));
                warcry_ears::resample::Resampler::new(DEVICE_RATE).process(&clip)
            })
            .collect();
        vec![learn("bobber", &examples, DEFAULT_FRAMES).unwrap()]
    }

    fn bites(events: &[Event]) -> usize {
        events
            .iter()
            .filter(|event| matches!(event, Event::Observation(Observation::Bite { .. })))
            .count()
    }

    #[test]
    fn a_cast_then_the_taught_sound_is_a_bite() {
        let mut pipeline = Pipeline::with_sounds(DEVICE_RATE, taught(), "bobber");
        let mut events = pipeline.push(&tone(FREQS[0], 150));
        events.extend(pipeline.push(&hush(1_200)));
        events.extend(pipeline.push(&splash()));
        events.extend(pipeline.push(&hush(200)));
        assert_eq!(events[0], Event::Beacon(Beacon::CastStarted));
        assert_eq!(bites(&events), 1, "{events:?}");
    }

    #[test]
    fn the_taught_sound_outside_a_cast_is_not_a_bite() {
        let mut pipeline = Pipeline::with_sounds(DEVICE_RATE, taught(), "bobber");
        let mut events = pipeline.push(&hush(1_200));
        events.extend(pipeline.push(&splash()));
        events.extend(pipeline.push(&hush(200)));
        assert_eq!(bites(&events), 0, "{events:?}");
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::Sound { .. })),
            "the sound should still be reported, just not as a bite"
        );
    }

    #[test]
    fn an_untaught_pipeline_never_reports_a_bite() {
        let mut pipeline = Pipeline::new(DEVICE_RATE);
        let mut events = pipeline.push(&tone(FREQS[0], 150));
        events.extend(pipeline.push(&hush(1_200)));
        events.extend(pipeline.push(&splash()));
        assert_eq!(bites(&events), 0);
    }

    #[test]
    fn a_beacon_beep_is_never_mistaken_for_a_bite() {
        let mut pipeline = Pipeline::with_sounds(DEVICE_RATE, taught(), "bobber");
        let mut events = pipeline.push(&tone(FREQS[0], 150));
        events.extend(pipeline.push(&hush(1_200)));
        events.extend(pipeline.push(&tone(FREQS[4], 150)));
        events.extend(pipeline.push(&tone(FREQS[6], 150)));
        events.extend(pipeline.push(&hush(400)));
        assert_eq!(bites(&events), 0, "{events:?}");
    }

    #[test]
    fn catching_a_fish_disarms_the_window() {
        let mut pipeline = Pipeline::with_sounds(DEVICE_RATE, taught(), "bobber");
        let mut events = pipeline.push(&tone(FREQS[0], 150));
        events.extend(pipeline.push(&hush(200)));
        events.extend(pipeline.push(&tone(FREQS[2], 150)));
        events.extend(pipeline.push(&hush(1_200)));
        events.extend(pipeline.push(&splash()));
        assert_eq!(bites(&events), 0, "{events:?}");
    }

    #[test]
    fn only_the_first_sound_in_a_cast_is_a_bite() {
        let mut pipeline = Pipeline::with_sounds(DEVICE_RATE, taught(), "bobber");
        let mut events = pipeline.push(&tone(FREQS[0], 150));
        events.extend(pipeline.push(&hush(1_200)));
        events.extend(pipeline.push(&splash()));
        events.extend(pipeline.push(&hush(800)));
        events.extend(pipeline.push(&splash()));
        events.extend(pipeline.push(&hush(200)));
        assert_eq!(bites(&events), 1, "{events:?}");
    }
}

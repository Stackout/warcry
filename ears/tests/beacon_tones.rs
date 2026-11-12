use std::path::PathBuf;
use std::process::Command;
use warcry_ears::resample::Resampler;
use warcry_ears::tone::{Beacon, Decoder, ToneCfg};
use warcry_ears::{FRAME, RATE};

const DEVICE_RATE: u32 = 48_000;

fn tone_path(symbol: u8) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../addon/WarcryBeacon/tones")
        .join(format!("t{symbol}.ogg"))
}

fn have_ffmpeg() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|out| out.status.success())
}

fn decode(symbol: u8) -> Vec<f32> {
    let output = Command::new("ffmpeg")
        .args(["-loglevel", "error", "-i"])
        .arg(tone_path(symbol))
        .args(["-f", "f32le", "-ac", "1", "-ar"])
        .arg(DEVICE_RATE.to_string())
        .arg("-")
        .output()
        .expect("ffmpeg failed to run");
    assert!(
        output.status.success(),
        "ffmpeg could not decode t{symbol}.ogg"
    );
    output
        .stdout
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

fn silence(ms: u32) -> Vec<f32> {
    vec![0.0; (DEVICE_RATE * ms / 1000) as usize]
}

fn through_pipeline(device_audio: &[f32]) -> Vec<Beacon> {
    let mut resampler = Resampler::new(DEVICE_RATE);
    let at_sixteen = resampler.process(device_audio);
    let mut decoder = Decoder::new(ToneCfg::default());
    at_sixteen
        .chunks(FRAME)
        .filter(|frame| frame.len() == FRAME)
        .filter_map(|frame| decoder.push(frame))
        .collect()
}

#[test]
fn every_shipped_tone_decodes_to_the_event_the_addon_means() {
    if !have_ffmpeg() {
        eprintln!("skipping: ffmpeg not available to decode the addon tones");
        return;
    }
    let expected = [
        (0u8, Beacon::CastStarted),
        (1, Beacon::CastEnded),
        (2, Beacon::Caught),
        (3, Beacon::BagsFull),
        (4, Beacon::LureExpiring),
        (5, Beacon::LureGone),
        (6, Beacon::Attention),
    ];
    for (symbol, want) in expected {
        let mut audio = silence(60);
        audio.extend(decode(symbol));
        audio.extend(silence(60));
        assert_eq!(
            through_pipeline(&audio),
            vec![want],
            "t{symbol}.ogg did not decode as {want:?}"
        );
    }
}

#[test]
fn the_identity_sequence_the_addon_plays_decodes_to_one_identity() {
    if !have_ffmpeg() {
        eprintln!("skipping: ffmpeg not available to decode the addon tones");
        return;
    }
    let mut audio = silence(60);
    for symbol in [7u8, 5, 2, 7] {
        audio.extend(decode(symbol));
        audio.extend(silence(80));
    }
    assert_eq!(through_pipeline(&audio), vec![Beacon::Identity(0o527)]);
}

#[test]
fn a_shipped_tone_is_still_found_under_loud_game_noise() {
    if !have_ffmpeg() {
        eprintln!("skipping: ffmpeg not available to decode the addon tones");
        return;
    }
    let mut state = 0x1234_5678_9abc_def0u64;
    let tone = decode(3);
    let noisy: Vec<f32> = tone
        .iter()
        .map(|sample| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let noise = (state >> 40) as f32 / 8_388_608.0 - 1.0;
            sample + noise * 0.08
        })
        .collect();
    let mut audio = silence(60);
    audio.extend(noisy);
    audio.extend(silence(60));
    assert_eq!(through_pipeline(&audio), vec![Beacon::BagsFull]);
}

#[test]
fn the_sample_rate_contract_holds() {
    assert_eq!(RATE, 16_000);
    assert_eq!(FRAME, 320);
}

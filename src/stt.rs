use anyhow::Result;
use std::path::Path;

pub trait Recognizer: Send {
    fn transcribe(&mut self, audio: &[f32], prompt: &str) -> Result<String>;
}

#[cfg(feature = "whisper")]
pub fn load(model: &Path, language: &str) -> Result<Box<dyn Recognizer>> {
    whisper::Whisper::load(model, language).map(|w| Box::new(w) as Box<dyn Recognizer>)
}

#[cfg(not(feature = "whisper"))]
pub fn load(_: &Path, _: &str) -> Result<Box<dyn Recognizer>> {
    anyhow::bail!("this build has no speech recognition; rebuild with `--features whisper`")
}

#[cfg(feature = "whisper")]
mod whisper {
    use super::Recognizer;
    use anyhow::{Context, Result};
    use std::path::Path;
    use whisper_rs::{
        FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
    };

    const MIN_SAMPLES: usize = 17_600;

    pub struct Whisper {
        state: WhisperState,
        language: String,
        threads: i32,
    }

    impl Whisper {
        pub fn load(model: &Path, language: &str) -> Result<Self> {
            whisper_rs::install_logging_hooks();
            let path = model.to_str().context("model path is not valid UTF-8")?;
            let ctx = WhisperContext::new_with_params(path, WhisperContextParameters::default())
                .with_context(|| format!("loading whisper model {}", model.display()))?;
            let threads = std::thread::available_parallelism()
                .map(|n| n.get().min(8))
                .unwrap_or(4) as i32;
            Ok(Self {
                state: ctx.create_state()?,
                language: language.to_string(),
                threads,
            })
        }
    }

    impl Recognizer for Whisper {
        fn transcribe(&mut self, audio: &[f32], prompt: &str) -> Result<String> {
            let padding = MIN_SAMPLES.saturating_sub(audio.len());
            let samples: Vec<f32> = audio
                .iter()
                .copied()
                .chain(std::iter::repeat_n(0.0, padding))
                .collect();
            let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
            params.set_language(Some(&self.language));
            params.set_n_threads(self.threads);
            params.set_single_segment(true);
            params.set_no_context(true);
            params.set_suppress_blank(true);
            params.set_print_progress(false);
            params.set_print_realtime(false);
            params.set_print_special(false);
            params.set_print_timestamps(false);
            params.set_initial_prompt(prompt);
            self.state.full(params, &samples)?;
            let text = self
                .state
                .as_iter()
                .map(|s| s.to_str_lossy().map(|t| t.trim().to_string()))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(text.join(" "))
        }
    }
}

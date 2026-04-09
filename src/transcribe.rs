use anyhow::{Context, Result};
use std::path::Path;
use tracing::{debug, info};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub struct Transcriber {
    ctx: WhisperContext,
}

impl Transcriber {
    pub fn new(model_path: &str) -> Result<Self> {
        if !Path::new(model_path).exists() {
            anyhow::bail!(
                "Model file not found: {}\n\
                Download one from: https://huggingface.co/ggerganov/whisper.cpp",
                model_path
            );
        }

        info!("Loading Whisper model from: {}", model_path);

        let mut params = WhisperContextParameters::default();
        // Disable GPU by default — works universally across all Linux setups.
        // Set TEN_FOUR_USE_GPU=1 to enable if you have CUDA/Vulkan support compiled in.
        params.use_gpu(
            std::env::var("TEN_FOUR_USE_GPU")
                .map(|v| v == "1" || v.to_lowercase() == "true")
                .unwrap_or(false),
        );

        let ctx = WhisperContext::new_with_params(model_path, params)
            .with_context(|| format!("Failed to load Whisper model from {}", model_path))?;

        info!("Whisper model loaded");
        Ok(Self { ctx })
    }

    /// Transcribe 16kHz mono f32 PCM samples loaded from a WAV file.
    pub fn transcribe_wav(&self, wav_path: &Path) -> Result<String> {
        let reader = hound::WavReader::open(wav_path)
            .with_context(|| format!("Failed to open WAV file: {:?}", wav_path))?;

        let spec = reader.spec();
        debug!("WAV spec: {:?}", spec);

        // whisper-rs expects f32 samples
        let samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => reader
                .into_samples::<f32>()
                .collect::<Result<_, _>>()?,
            hound::SampleFormat::Int => {
                let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
                reader
                    .into_samples::<i32>()
                    .map(|s| s.map(|v| v as f32 / max))
                    .collect::<Result<_, _>>()?
            }
        };

        self.transcribe_samples(&samples)
    }

    /// Transcribe raw 16kHz mono f32 PCM samples directly (no file needed).
    pub fn transcribe_samples(&self, samples: &[f32]) -> Result<String> {
        if samples.is_empty() {
            return Ok(String::new());
        }

        let mut state = self.ctx.create_state()
            .context("Failed to create Whisper state")?;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

        // Tuning for low-latency dictation
        params.set_n_threads(num_threads());
        params.set_translate(false);
        params.set_language(Some("auto"));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        // Strip leading/trailing whitespace and [BLANK_AUDIO] tokens
        params.set_no_context(true);

        info!("Transcribing {} samples...", samples.len());
        state
            .full(params, samples)
            .context("Whisper transcription failed")?;

        let num_segments = state.full_n_segments()?;
        let mut text = String::new();

        for i in 0..num_segments {
            let segment = state.full_get_segment_text(i)?;
            let trimmed = segment.trim();
            // Filter out common hallucination artifacts
            if !trimmed.is_empty()
                && !trimmed.eq_ignore_ascii_case("[blank_audio]")
                && !trimmed.eq_ignore_ascii_case("(silence)")
            {
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(trimmed);
            }
        }

        info!("Transcription: {:?}", text);
        Ok(text)
    }
}

fn num_threads() -> i32 {
    // Use half the available logical cores, min 1, max 4
    // Keeps the Pi from thermal throttling during inference
    let cpus = num_cpus();
    ((cpus / 2).max(1)).min(4) as i32
}

fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

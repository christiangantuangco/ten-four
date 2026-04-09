use anyhow::{Context, Result};
use std::path::Path;
use std::sync::Mutex;
use tracing::{debug, info};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState};

// ── Trait ─────────────────────────────────────────────────────────────────────

pub trait TranscribeEngine: Send + Sync {
    fn transcribe_samples(&self, samples: &[f32]) -> Result<String>;
}

// ── Whisper ───────────────────────────────────────────────────────────────────

struct StateCell(WhisperState<'static>);
unsafe impl Send for StateCell {}

pub struct WhisperTranscriber {
    // Drop order: state before _ctx
    state: Mutex<StateCell>,
    _ctx: Box<WhisperContext>,
}

impl WhisperTranscriber {
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
        params.use_gpu(
            std::env::var("TEN_FOUR_USE_GPU")
                .map(|v| v == "1" || v.to_lowercase() == "true")
                .unwrap_or(false),
        );

        let ctx = Box::new(
            WhisperContext::new_with_params(model_path, params)
                .with_context(|| format!("Failed to load Whisper model from {}", model_path))?,
        );

        // Safety: ctx is heap-allocated and never moves. State borrows from ctx;
        // correct drop order guaranteed by field declaration order (state before _ctx).
        let state = unsafe {
            let ctx_static: &'static WhisperContext = &*(&*ctx as *const WhisperContext);
            ctx_static.create_state().context("Failed to create Whisper state")?
        };

        info!("Whisper model loaded: {}", model_path);
        Ok(Self {
            state: Mutex::new(StateCell(state)),
            _ctx: ctx,
        })
    }
}

impl TranscribeEngine for WhisperTranscriber {
    fn transcribe_samples(&self, samples: &[f32]) -> Result<String> {
        if samples.is_empty() {
            return Ok(String::new());
        }

        let mut guard = self.state.lock().unwrap();
        let state = &mut guard.0;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_n_threads(num_threads());
        params.set_translate(false);
        params.set_language(Some("en"));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_no_context(true);

        info!("Transcribing {} samples (whisper)...", samples.len());
        state.full(params, samples).context("Whisper transcription failed")?;

        let num_segments = state.full_n_segments()?;
        let mut text = String::new();

        for i in 0..num_segments {
            let segment = state.full_get_segment_text(i)?;
            let trimmed = segment.trim();
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

// ── Vosk ──────────────────────────────────────────────────────────────────────

pub struct VoskTranscriber {
    model: vosk::Model,
}

impl VoskTranscriber {
    pub fn new(model_path: &str) -> Result<Self> {
        if !Path::new(model_path).exists() {
            anyhow::bail!(
                "Vosk model directory not found: {}\n\
                Download one from: https://alphacephei.com/vosk/models",
                model_path
            );
        }

        info!("Loading Vosk model from: {}", model_path);
        let model = vosk::Model::new(model_path)
            .ok_or_else(|| anyhow::anyhow!("Failed to load Vosk model from {}", model_path))?;
        info!("Vosk model loaded: {}", model_path);
        Ok(Self { model })
    }
}

impl TranscribeEngine for VoskTranscriber {
    fn transcribe_samples(&self, samples: &[f32]) -> Result<String> {
        if samples.is_empty() {
            return Ok(String::new());
        }

        // Vosk expects i16 samples at 16kHz
        let samples_i16: Vec<i16> = samples
            .iter()
            .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
            .collect();

        let mut recognizer = vosk::Recognizer::new(&self.model, 16000.0)
            .ok_or_else(|| anyhow::anyhow!("Failed to create Vosk recognizer"))?;
        recognizer.set_max_alternatives(0);
        recognizer.set_words(false);

        info!("Transcribing {} samples (vosk)...", samples.len());
        recognizer.accept_waveform(&samples_i16);
        let result = recognizer.final_result().single().map(|r| r.text.to_string()).unwrap_or_default();

        let text = result.trim().to_string();
        info!("Transcription: {:?}", text);
        Ok(text)
    }
}

// ── Shared helper used by daemon ──────────────────────────────────────────────

/// Transcribe a WAV file using whichever engine is provided.
pub fn transcribe_wav(engine: &dyn TranscribeEngine, wav_path: &Path) -> Result<String> {
    let reader = hound::WavReader::open(wav_path)
        .with_context(|| format!("Failed to open WAV file: {:?}", wav_path))?;

    let spec = reader.spec();
    debug!("WAV spec: {:?}", spec);

    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.into_samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .into_samples::<i32>()
                .map(|s| s.map(|v| v as f32 / max))
                .collect::<Result<_, _>>()?
        }
    };

    engine.transcribe_samples(&samples)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn num_threads() -> i32 {
    let cpus = num_cpus();
    ((cpus / 2).max(1)).min(4) as i32
}

fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

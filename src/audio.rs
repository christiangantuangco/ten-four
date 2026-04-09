use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::{debug, info, warn};

pub struct AudioRecorder {
    sample_rate: u32,
    channels: u16,
}

impl AudioRecorder {
    pub fn new() -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .context("No input audio device found. Check your microphone is connected.")?;

        info!("Using audio device: {}", device.name().unwrap_or_default());

        let config = device
            .default_input_config()
            .context("Failed to get default input config")?;

        Ok(Self {
            sample_rate: config.sample_rate().0,
            channels: config.channels(),
        })
    }

    /// Record audio until `stop_signal` is set to true.
    /// Returns raw f32 PCM samples.
    pub fn record_until_stop(&self, stop_signal: Arc<Mutex<bool>>) -> Result<Vec<f32>> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .context("No input audio device found")?;

        let config = device.default_input_config()?;
        debug!("Input config: {:?}", config);

        let samples: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
        let samples_clone = Arc::clone(&samples);

        let err_fn = |err| warn!("Audio stream error: {}", err);

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => {
                let s = samples_clone.clone();
                device.build_input_stream(
                    &config.into(),
                    move |data: &[f32], _| {
                        s.lock().unwrap().extend_from_slice(data);
                    },
                    err_fn,
                    None,
                )?
            }
            cpal::SampleFormat::I16 => {
                let s = samples_clone.clone();
                device.build_input_stream(
                    &config.into(),
                    move |data: &[i16], _| {
                        let converted: Vec<f32> =
                            data.iter().map(|&x| x as f32 / i16::MAX as f32).collect();
                        s.lock().unwrap().extend_from_slice(&converted);
                    },
                    err_fn,
                    None,
                )?
            }
            cpal::SampleFormat::U16 => {
                let s = samples_clone.clone();
                device.build_input_stream(
                    &config.into(),
                    move |data: &[u16], _| {
                        let converted: Vec<f32> = data
                            .iter()
                            .map(|&x| (x as f32 / u16::MAX as f32) * 2.0 - 1.0)
                            .collect();
                        s.lock().unwrap().extend_from_slice(&converted);
                    },
                    err_fn,
                    None,
                )?
            }
            fmt => anyhow::bail!("Unsupported sample format: {:?}", fmt),
        };

        stream.play().context("Failed to start audio stream")?;
        info!("🎙  Recording...");

        // Poll until stop signal is set
        loop {
            std::thread::sleep(Duration::from_millis(50));
            if *stop_signal.lock().unwrap() {
                break;
            }
        }

        drop(stream);
        info!("⏹  Recording stopped");

        let recorded = samples.lock().unwrap().clone();
        info!("Captured {} samples ({:.1}s)", recorded.len(), recorded.len() as f32 / self.sample_rate as f32);

        Ok(recorded)
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }
}

/// Downsample and convert stereo → mono f32 PCM to a WAV file at 16kHz (required by Whisper).
pub fn write_wav(
    path: &std::path::Path,
    samples: &[f32],
    src_sample_rate: u32,
    src_channels: u16,
) -> Result<()> {
    const TARGET_RATE: u32 = 16_000;

    // Mix down to mono
    let mono: Vec<f32> = if src_channels == 1 {
        samples.to_vec()
    } else {
        samples
            .chunks(src_channels as usize)
            .map(|frame| frame.iter().sum::<f32>() / src_channels as f32)
            .collect()
    };

    // Simple linear interpolation resample to 16kHz
    let resampled = if src_sample_rate != TARGET_RATE {
        let ratio = src_sample_rate as f64 / TARGET_RATE as f64;
        let out_len = (mono.len() as f64 / ratio) as usize;
        (0..out_len)
            .map(|i| {
                let src_pos = i as f64 * ratio;
                let src_idx = src_pos as usize;
                let frac = src_pos - src_idx as f64;
                let a = mono.get(src_idx).copied().unwrap_or(0.0);
                let b = mono.get(src_idx + 1).copied().unwrap_or(0.0);
                (a + (b - a) * frac as f32) as f32
            })
            .collect()
    } else {
        mono
    };

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: TARGET_RATE,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };

    let mut writer = hound::WavWriter::create(path, spec)
        .with_context(|| format!("Failed to create WAV file at {:?}", path))?;

    for sample in &resampled {
        writer.write_sample(*sample)?;
    }

    writer.finalize()?;
    debug!("WAV written: {:?} ({} samples @ 16kHz)", path, resampled.len());
    Ok(())
}

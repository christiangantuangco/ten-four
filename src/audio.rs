use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::{debug, info, warn};

pub struct AudioRecorder {
    sample_rate: u32,
    channels: u16,
    device: Option<String>,
}

impl AudioRecorder {
    pub fn new(device: Option<String>) -> Result<Self> {
        if device.is_some() {
            // parec will record at 16kHz mono — no cpal needed
            return Ok(Self { sample_rate: 16_000, channels: 1, device });
        }
        let (dev, config) = find_input_device()?;
        info!("Using audio device: {}", dev.name().unwrap_or_default());
        Ok(Self {
            sample_rate: config.sample_rate().0,
            channels: config.channels(),
            device: None,
        })
    }

    /// Record audio until `stop_signal` is set to true.
    /// Returns raw f32 PCM samples.
    pub fn record_until_stop(&self, stop_signal: Arc<Mutex<bool>>) -> Result<Vec<f32>> {
        if let Some(ref dev_name) = self.device {
            return record_parec(dev_name, stop_signal);
        }

        let (device, config) = find_input_device()?;
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

/// Record from a PipeWire/PulseAudio source via `parec`.
/// Captures float32le at 16kHz mono — no resampling needed downstream.
fn record_parec(device: &str, stop_signal: Arc<Mutex<bool>>) -> Result<Vec<f32>> {
    use std::io::Read;

    let mut child = std::process::Command::new("parec")
        .args([
            &format!("--device={}", device),
            "--format=float32le",
            "--rate=16000",
            "--channels=1",
            "--raw",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("Failed to spawn parec. Is PipeWire/PulseAudio running?")?;

    info!("Recording via PipeWire (parec) from {}", device);

    let samples: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let samples_clone = Arc::clone(&samples);
    let mut stdout = child.stdout.take().context("Failed to get parec stdout")?;

    // Reader thread: converts raw float32le bytes → f32 samples
    let reader = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match stdout.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let floats: Vec<f32> = buf[..n]
                        .chunks_exact(4)
                        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                        .collect();
                    samples_clone.lock().unwrap().extend_from_slice(&floats);
                }
            }
        }
    });

    loop {
        std::thread::sleep(Duration::from_millis(50));
        if *stop_signal.lock().unwrap() {
            break;
        }
    }

    // Kill parec — causes EOF on stdout, ending the reader thread
    let _ = child.kill();
    let _ = child.wait();
    let _ = reader.join();

    let recorded = samples.lock().unwrap().clone();
    info!("Captured {} samples ({:.1}s)", recorded.len(), recorded.len() as f32 / 16_000.0);

    Ok(recorded)
}

/// Find a usable input device + config.
/// Tries the system default first; if its config is invalid, falls back to
/// enumerating all input devices and picking the first one that works.
fn find_input_device() -> Result<(cpal::Device, cpal::SupportedStreamConfig)> {
    let host = cpal::default_host();

    if let Some(device) = host.default_input_device() {
        if let Ok(config) = device.default_input_config() {
            return Ok((device, config));
        }
        warn!(
            "Default input device '{}' has no usable config, scanning all devices...",
            device.name().unwrap_or_default()
        );
    }

    for device in host.input_devices().context("Failed to enumerate input devices")? {
        if let Ok(config) = device.default_input_config() {
            return Ok((device, config));
        }
    }

    anyhow::bail!(
        "No usable audio input device found. Check your microphone is connected.\n\
        List devices with: arecord -l"
    )
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

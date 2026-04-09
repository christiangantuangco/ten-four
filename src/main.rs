mod audio;
mod daemon;
mod inject;
mod ipc;
mod transcribe;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use std::sync::Arc;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Clone, ValueEnum)]
enum Engine {
    Whisper,
    Vosk,
}

#[derive(Parser)]
#[command(
    name = "ten-four",
    about = "Push-to-talk speech-to-text for Linux",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the ten-four daemon (run this in the background / as a systemd service)
    Daemon {
        /// Transcription engine to use
        #[arg(long, default_value = "whisper")]
        engine: Engine,

        /// Path to the Whisper GGML model file (whisper engine)
        #[arg(long, env = "TEN_FOUR_MODEL")]
        model: Option<String>,

        /// Path to the Vosk model directory (vosk engine)
        #[arg(long, env = "VOSK_MODEL")]
        vosk_model: Option<String>,

        /// Unix socket path (default: /tmp/ten-four.sock)
        #[arg(long, default_value = ipc::SOCKET_PATH)]
        socket: String,

        /// Injection method: ydotool or xdotool
        #[arg(long, default_value = "ydotool")]
        injector: String,

        /// Microphone source name to use (from `ten-four list-mics`)
        #[arg(long, env = "DEVICE")]
        device: Option<String>,
    },

    /// Toggle recording on/off (bind this to a hotkey in your DE)
    Toggle {
        /// Unix socket path (default: /tmp/voicetype.sock)
        #[arg(long, default_value = ipc::SOCKET_PATH)]
        socket: String,
    },

    /// Print the status of the daemon (idle / recording)
    Status {
        /// Unix socket path (default: /tmp/voicetype.sock)
        #[arg(long, default_value = ipc::SOCKET_PATH)]
        socket: String,
    },

    /// Test hotkey toggle without audio — just logs state changes
    TestHotkey {
        /// Unix socket path (default: /tmp/voicetype.sock)
        #[arg(long, default_value = ipc::SOCKET_PATH)]
        socket: String,
    },

    /// Print an example systemd user service file to stdout
    InstallService,

    /// List available microphone input devices (ALSA + PulseAudio/PipeWire)
    ListMics,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("ten_four=info".parse()?))
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::Daemon {
            engine,
            model,
            vosk_model,
            socket,
            injector,
            device,
        } => {
            let resolved_device = device.map(|d| resolve_device_name(&d)).transpose()?;
            info!("Starting ten-four daemon");
            info!("Socket: {}", socket);
            info!("Injector: {}", injector);
            if let Some(ref d) = resolved_device {
                info!("Device: {}", d);
            }

            let transcriber: Arc<dyn transcribe::TranscribeEngine> = match engine {
                Engine::Whisper => {
                    let model_path = resolve_model_path(model)?;
                    let t = tokio::task::spawn_blocking(move || {
                        transcribe::WhisperTranscriber::new(&model_path)
                    })
                    .await??;
                    Arc::new(t)
                }
                Engine::Vosk => {
                    let model_path = vosk_model.ok_or_else(|| {
                        anyhow::anyhow!("--vosk-model <path> is required when using --engine vosk\n\
                            Download a model from: https://alphacephei.com/vosk/models")
                    })?;
                    let t = tokio::task::spawn_blocking(move || {
                        transcribe::VoskTranscriber::new(&model_path)
                    })
                    .await??;
                    Arc::new(t)
                }
            };

            daemon::run(transcriber, socket, injector, resolved_device).await?;
        }

        Command::TestHotkey { socket } => {
            daemon::run_test_hotkey(socket).await?;
        }

        Command::Toggle { socket } => {
            ipc::send_command(&socket, ipc::Command::Toggle).await?;
        }

        Command::Status { socket } => {
            let status = ipc::send_command(&socket, ipc::Command::Status).await?;
            println!("{}", status);
        }

        Command::InstallService => {
            print_service_file();
        }

        Command::ListMics => {
            list_mics();
        }
    }

    Ok(())
}

fn resolve_model_path(model: Option<String>) -> Result<String> {
    if let Some(m) = model {
        return Ok(m);
    }

    // Scan the XDG default models directory and pick the smallest .bin file
    if let Some(models_dir) = dirs::data_dir().map(|d| d.join("ten-four/models")) {
        if let Ok(entries) = std::fs::read_dir(&models_dir) {
            let mut models: Vec<(u64, std::path::PathBuf)> = entries
                .flatten()
                .filter_map(|e| {
                    let path = e.path();
                    if path.extension().and_then(|x| x.to_str()) == Some("bin") {
                        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(u64::MAX);
                        Some((size, path))
                    } else {
                        None
                    }
                })
                .collect();

            models.sort_by_key(|(size, _)| *size);

            if let Some((_, path)) = models.into_iter().next() {
                let path_str = path.to_string_lossy().to_string();
                info!("Auto-detected model: {}", path_str);
                return Ok(path_str);
            }
        }
    }

    anyhow::bail!(
        "No model file found.\n\n\
        Options:\n  \
        1. Pass it directly:       ten-four daemon --model /path/to/model.bin\n  \
        2. Set an env var:         TEN_FOUR_MODEL=/path/to/model.bin ten-four daemon\n  \
        3. Drop it in the default dir: ~/.local/share/ten-four/models/<any>.bin\n\n\
        Download a model:\n  \
        mkdir -p ~/.local/share/ten-four/models\n  \
        wget -O ~/.local/share/ten-four/models/ggml-base.bin \\\n    \
        https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin"
    )
}

fn list_mics() {
    use cpal::traits::{DeviceTrait, HostTrait};

    println!("ALSA input devices:");
    let host = cpal::default_host();
    let default_name = host
        .default_input_device()
        .and_then(|d| d.name().ok())
        .unwrap_or_default();

    match host.input_devices() {
        Ok(devices) => {
            let mut found = false;
            for device in devices {
                let name = device.name().unwrap_or_else(|_| "<unknown>".to_string());
                let marker = if name == default_name { " (default)" } else { "" };
                println!("  {}{}", name, marker);
                found = true;
            }
            if !found {
                println!("  (none found)");
            }
        }
        Err(e) => println!("  Failed to enumerate ALSA devices: {}", e),
    }

    println!();
    println!("PulseAudio / PipeWire sources (includes Bluetooth):");
    match std::process::Command::new("pactl").args(["list", "sources"]).output() {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            let sources = parse_pactl_sources(&text);
            if sources.is_empty() {
                println!("  (none found)");
            } else {
                for (name, description) in &sources {
                    println!("  {:<55} {}", name, description);
                }
            }
        }
        Ok(_) => println!("  pactl returned an error — is PulseAudio/PipeWire running?"),
        Err(_) => println!("  pactl not found — install pulseaudio-utils or pipewire-pulse"),
    }

    println!();
    println!("To use a specific device, pass the source name to the daemon:");
    println!("  ten-four daemon --device <source-name>");
    println!("  DEVICE=<source-name> ten-four daemon");
}

/// Resolve a device string (friendly name or source name) to the actual pactl source name.
fn resolve_device_name(input: &str) -> Result<String> {
    let output = std::process::Command::new("pactl")
        .args(["list", "sources"])
        .output()
        .map_err(|_| anyhow::anyhow!("pactl not found — install pulseaudio-utils or pipewire-pulse"))?;

    let text = String::from_utf8_lossy(&output.stdout);
    let sources = parse_pactl_sources(&text);

    // Match against source name (exact) or description (case-insensitive substring)
    for (name, description) in &sources {
        if name == input || description.to_lowercase().contains(&input.to_lowercase()) {
            return Ok(name.clone());
        }
    }

    anyhow::bail!(
        "Device '{}' not found. Run `ten-four list-mics` to see available devices.",
        input
    )
}

/// Parse `pactl list sources` output into (name, description) pairs.
/// Skips monitor (loopback) sources.
fn parse_pactl_sources(text: &str) -> Vec<(String, String)> {
    let mut sources = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_desc: Option<String> = None;

    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(name) = trimmed.strip_prefix("Name: ") {
            // Save previous source if complete
            if let (Some(n), Some(d)) = (current_name.take(), current_desc.take()) {
                if !n.contains(".monitor") {
                    sources.push((n, d));
                }
            }
            current_name = Some(name.to_string());
            current_desc = None;
        } else if let Some(desc) = trimmed.strip_prefix("Description: ") {
            current_desc = Some(desc.to_string());
        }
    }
    // Flush last entry
    if let (Some(n), Some(d)) = (current_name, current_desc) {
        if !n.contains(".monitor") {
            sources.push((n, d));
        }
    }

    sources
}

fn print_service_file() {
    let exe = std::env::current_exe()
        .unwrap_or_else(|_| "/usr/local/bin/ten-four".into())
        .to_string_lossy()
        .to_string();

    println!(
        r#"[Unit]
Description=ten-four speech-to-text daemon
After=graphical-session.target

[Service]
Type=simple
ExecStart={exe} daemon
Restart=on-failure
RestartSec=3
# Optional: set TEN_FOUR_MODEL=/path/to/model.bin to pin a specific model.
# Otherwise, ten-four will auto-detect any .bin in ~/.local/share/ten-four/models/
Environment=RUST_LOG=ten_four=info

[Install]
WantedBy=default.target
"#,
        exe = exe
    );

    eprintln!("→ Save to: ~/.config/systemd/user/ten-four.service");
    eprintln!("→ Then run: systemctl --user daemon-reload && systemctl --user enable --now ten-four");
}

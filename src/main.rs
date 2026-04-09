mod audio;
mod daemon;
mod inject;
mod ipc;
mod transcribe;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::info;
use tracing_subscriber::EnvFilter;

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
        /// Path to the whisper.cpp GGML model file
        #[arg(long, env = "TEN_FOUR_MODEL")]
        model: Option<String>,

        /// Unix socket path (default: /tmp/voicetype.sock)
        #[arg(long, default_value = ipc::SOCKET_PATH)]
        socket: String,

        /// Injection method: ydotool or xdotool
        #[arg(long, default_value = "ydotool")]
        injector: String,
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

    /// Print an example systemd user service file to stdout
    InstallService,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("ten_four=info".parse()?))
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::Daemon {
            model,
            socket,
            injector,
        } => {
            let model_path = resolve_model_path(model)?;
            info!("Starting ten-four daemon");
            info!("Model: {}", model_path);
            info!("Socket: {}", socket);
            info!("Injector: {}", injector);
            daemon::run(model_path, socket, injector).await?;
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
    }

    Ok(())
}

fn resolve_model_path(model: Option<String>) -> Result<String> {
    if let Some(m) = model {
        return Ok(m);
    }

    // Scan the XDG default models directory for any .bin file
    if let Some(models_dir) = dirs::data_dir().map(|d| d.join("ten-four/models")) {
        if let Ok(entries) = std::fs::read_dir(&models_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("bin") {
                    let path_str = path.to_string_lossy().to_string();
                    info!("Auto-detected model: {}", path_str);
                    return Ok(path_str);
                }
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

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

    // Check default locations
    let candidates = vec![
        // Project-local models dir
        "./models/ggml-small.bin".to_string(),
        "./models/ggml-tiny.bin".to_string(),
        // XDG data dir
        dirs::data_dir()
            .map(|d| d.join("ten-four/models/ggml-small.bin").to_string_lossy().to_string())
            .unwrap_or_default(),
        dirs::data_dir()
            .map(|d| d.join("ten-four/models/ggml-tiny.bin").to_string_lossy().to_string())
            .unwrap_or_default(),
    ];

    for path in &candidates {
        if !path.is_empty() && std::path::Path::new(path).exists() {
            return Ok(path.clone());
        }
    }

    anyhow::bail!(
        "No model file found. Download a model and pass it with --model, \
        set TEN_FOUR_MODEL env var, or place it at ./models/ggml-small.bin\n\n\
        Download: https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin"
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
Environment=TEN_FOUR_MODEL=%h/.local/share/ten-four/models/ggml-small.bin
Environment=RUST_LOG=ten_four=info

[Install]
WantedBy=default.target
"#,
        exe = exe
    );

    eprintln!("→ Save to: ~/.config/systemd/user/ten-four.service");
    eprintln!("→ Then run: systemctl --user daemon-reload && systemctl --user enable --now ten-four");
}

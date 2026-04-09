use anyhow::{Context, Result};
use std::process::Command;
use tracing::{debug, warn};

#[derive(Debug, Clone)]
pub enum Injector {
    Ydotool,
    Xdotool,
}

impl Injector {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "xdotool" => Injector::Xdotool,
            _ => Injector::Ydotool,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Injector::Ydotool => "ydotool",
            Injector::Xdotool => "xdotool",
        }
    }

    /// Check if the required binary is in PATH and give a helpful error if not.
    pub fn check_available(&self) -> Result<()> {
        let binary = self.name();
        let status = Command::new("which")
            .arg(binary)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if !status {
            let install_hint = match self {
                Injector::Ydotool => {
                    "Install ydotool:\n  Debian/Ubuntu: sudo apt install ydotool\n  Fedora: sudo dnf install ydotool\n  Arch: sudo pacman -S ydotool\n\nThen start the daemon:\n  systemctl --user enable --now ydotool"
                }
                Injector::Xdotool => {
                    "Install xdotool:\n  Debian/Ubuntu: sudo apt install xdotool\n  Fedora: sudo dnf install xdotool\n  Arch: sudo pacman -S xdotool\n\nNote: xdotool only works on X11, not Wayland."
                }
            };
            anyhow::bail!("`{}` not found in PATH.\n\n{}", binary, install_hint);
        }

        // Additional ydotool-specific check: verify the daemon socket exists
        if let Injector::Ydotool = self {
            check_ydotool_daemon()?;
        }

        Ok(())
    }

    /// Type `text` into the currently focused window.
    pub fn type_text(&self, text: &str) -> Result<()> {
        if text.is_empty() {
            debug!("Empty text, nothing to inject");
            return Ok(());
        }

        debug!("Injecting {} chars via {}", text.len(), self.name());

        match self {
            Injector::Ydotool => {
                // Small delay to ensure focus is back on the target window
                // after the user released the hotkey
                std::thread::sleep(std::time::Duration::from_millis(150));

                let status = Command::new("ydotool")
                    .arg("type")
                    .arg("--")
                    .arg(text)
                    .status()
                    .context("Failed to run ydotool")?;

                if !status.success() {
                    anyhow::bail!("ydotool exited with status: {}", status);
                }
            }

            Injector::Xdotool => {
                std::thread::sleep(std::time::Duration::from_millis(150));

                let status = Command::new("xdotool")
                    .arg("type")
                    .arg("--clearmodifiers")
                    .arg("--")
                    .arg(text)
                    .status()
                    .context("Failed to run xdotool")?;

                if !status.success() {
                    anyhow::bail!("xdotool exited with status: {}", status);
                }
            }
        }

        Ok(())
    }
}

fn check_ydotool_daemon() -> Result<()> {
    // ydotool looks for /tmp/.ydotool_socket or the YDOTOOL_SOCKET env var
    let socket_path = std::env::var("YDOTOOL_SOCKET")
        .unwrap_or_else(|_| "/tmp/.ydotool_socket".to_string());

    if !std::path::Path::new(&socket_path).exists() {
        warn!("ydotoold socket not found at {}. The ydotool daemon may not be running.", socket_path);
        warn!("Start it with: systemctl --user enable --now ydotool");
        // Don't hard-fail here — some setups use a different socket path
        // The actual type command will fail with a clear error if it can't connect
    }

    Ok(())
}

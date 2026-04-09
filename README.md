# ten-four

Push-to-talk speech-to-text for Linux. Press a hotkey → speak → release → text appears at your cursor. Works in terminals, browsers, code editors, anywhere.

- **Fully offline** — powered by [whisper.cpp](https://github.com/ggerganov/whisper.cpp) via [`whisper-rs`](https://github.com/tazz4843/whisper-rs)
- **Wayland + X11** — text injection via `ydotool` (Wayland-native) or `xdotool` (X11)
- **Single binary** — no Python, no venvs, no runtime dependencies beyond `ydotool`
- **aarch64 ready** — tested on Raspberry Pi (aarch64 Debian)

---

## How It Works

```
ten-four daemon   ← runs in background, owns audio + whisper model
ten-four toggle   ← bind this to a hotkey in your DE
```

The daemon listens on a Unix socket (`/tmp/ten-four.sock`). Each call to `ten-four toggle` connects to the socket and flips the daemon between **idle → recording → transcribing → idle**.

---

## 1. Install Build Dependencies

### Debian / Ubuntu / Raspberry Pi OS

```bash
sudo apt update
sudo apt install -y \
  build-essential cmake libclang-dev \
  libasound2-dev \
  ydotool \
  pkg-config curl git
```

### Fedora

```bash
sudo dnf install -y \
  gcc cmake clang-devel \
  alsa-lib-devel \
  ydotool \
  pkg-config curl git
```

### Arch Linux / Manjaro

```bash
sudo pacman -S \
  base-devel cmake clang \
  alsa-lib \
  ydotool \
  pkg-config curl git
```

### openSUSE

```bash
sudo zypper install -y \
  gcc cmake clang-devel \
  alsa-devel \
  ydotool \
  pkg-config curl git
```

---

## 2. Install Rust

If you don't have Rust installed:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

---

## 3. Build ten-four

```bash
git clone https://github.com/yourname/ten-four.git
cd ten-four
cargo build --release
```

The binary will be at `./target/release/ten-four`. Optionally install it:

```bash
sudo cp target/release/ten-four /usr/local/bin/ten-four
```

---

## 4. Download a Whisper Model

ten-four resolves the model in this order:

1. `--model /path/to/model.bin` flag (explicit, highest priority)
2. `TEN_FOUR_MODEL=/path/to/model.bin` environment variable
3. Auto-detect: any `.bin` file found in `~/.local/share/ten-four/models/`
4. Error with instructions if nothing is found

For most users, dropping a model into the default directory is the easiest setup:

```bash
mkdir -p ~/.local/share/ten-four/models
cd ~/.local/share/ten-four/models

# Recommended: base model — good balance of speed and accuracy
wget https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin

# Or pick another size:
wget https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin
wget https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin
```

| Model | Size  | Speed (CPU) | Accuracy           |
|-------|-------|-------------|--------------------|
| tiny  | 75MB  | ~1s         | Good               |
| base  | 142MB | ~2s         | Better             |
| small | 466MB | ~4s         | Best for dictation |

If you prefer to store the model elsewhere, pass it explicitly:

```bash
ten-four daemon --model /path/to/your/model.bin
# or
TEN_FOUR_MODEL=/path/to/your/model.bin ten-four daemon
```

---

## 5. Set Up ydotoold (Wayland)

`ydotool` requires a background daemon (`ydotoold`) to inject keystrokes.

```bash
# Add yourself to the input group (required once, then log out/in)
sudo usermod -aG input $USER

# Enable the ydotool systemd user service
systemctl --user enable --now ydotool
```

> **Note:** If `systemctl --user enable ydotool` fails, your distro may not ship a unit file. Run it manually: `ydotoold &`

### X11 users

Use `xdotool` instead — no daemon needed:

```bash
# Debian/Ubuntu
sudo apt install xdotool

# Fedora
sudo dnf install xdotool

# Arch
sudo pacman -S xdotool
```

Pass `--injector xdotool` when starting the daemon (see step 6).

---

## 6. Install as a systemd User Service

Generate and install the service file:

```bash
ten-four install-service > ~/.config/systemd/user/ten-four.service

# Review it (optional)
cat ~/.config/systemd/user/ten-four.service

# Enable and start
systemctl --user daemon-reload
systemctl --user enable --now ten-four

# Check it's running
systemctl --user status ten-four
```

The generated service relies on auto-detection — it will pick up any `.bin` file in `~/.local/share/ten-four/models/`.  
To pin a specific model, add `Environment=TEN_FOUR_MODEL=/path/to/model.bin` to the service file. To use xdotool, append `--injector xdotool` to the `ExecStart` line.

---

## 7. Bind the Hotkey

Bind `ten-four toggle` to a key in your desktop environment:

### GNOME
Settings → Keyboard → View and Customize Shortcuts → Custom Shortcuts → `+`
- **Name:** Ten Four
- **Command:** `ten-four toggle`
- **Shortcut:** e.g. `Super+Space`

### KDE Plasma
System Settings → Shortcuts → Custom Shortcuts → Edit → New → Global Shortcut → Command/URL
- **Command:** `ten-four toggle`
- **Trigger:** your key combo

### Sway / Hyprland / i3 (config file)
```
# Sway / i3
bindsym $mod+space exec ten-four toggle

# Hyprland
bind = SUPER, SPACE, exec, ten-four toggle
```

### sxhkd (distro-agnostic hotkey daemon)
```
super + space
    ten-four toggle
```

---

## Usage

Once the daemon is running and the hotkey is bound:

1. Click into any text field (terminal, browser, editor, etc.)
2. Press your hotkey → you'll see `🎙 Recording...` in the daemon logs
3. Speak clearly
4. Press the hotkey again → transcription runs → text is typed at your cursor

Check the daemon status at any time:

```bash
ten-four status
# → idle | recording | transcribing
```

---

## Configuration

| Method | Description |
|--------|-------------|
| `--model PATH` | Explicit path to any GGML `.bin` model file |
| `TEN_FOUR_MODEL=PATH` | Same as `--model`, via environment variable |
| _(neither set)_ | Auto-detects any `.bin` in `~/.local/share/ten-four/models/` |
| `--injector ydotool\|xdotool` | Text injection method |
| `TEN_FOUR_USE_GPU=1` | Enable GPU inference (requires CUDA/Vulkan build) |
| `RUST_LOG=ten_four=debug` | Verbose logging |

---

## Troubleshooting

### "No input audio device found"
Your microphone isn't detected. Check:
```bash
arecord -l          # list capture devices
pactl list sources  # PulseAudio
```

### "Could not connect to ten-four daemon"
The daemon isn't running. Start it:
```bash
systemctl --user start ten-four
# or manually:
ten-four daemon
```

### Text isn't appearing / ydotool errors
```bash
# Check ydotoold is running
systemctl --user status ydotool

# Check you're in the input group
groups | grep input
# If not: sudo usermod -aG input $USER  (then log out and back in)
```

### Slow transcription on Raspberry Pi
Use the `tiny` model and set `RUST_LOG=ten_four=info` to see timing.  
The daemon auto-detects CPU count and caps inference threads at 4.

### Wayland: text appears in wrong window
The 150ms delay in `inject.rs` is intentional — it waits for focus to return to your target window after the hotkey. Increase it if your DE is slower:
```bash
# Edit src/inject.rs: change 150 to 300
# Then: cargo build --release
```

---

## Project Structure

```
ten-four/
├── src/
│   ├── main.rs         # CLI entry: daemon | toggle | status | install-service
│   ├── daemon.rs       # Main loop: state machine + task orchestration
│   ├── audio.rs        # cpal recording + WAV conversion (→ 16kHz mono)
│   ├── transcribe.rs   # whisper-rs inference wrapper
│   ├── inject.rs       # ydotool / xdotool text injection
│   └── ipc.rs          # Unix socket client + server helpers
├── models/             # Place .bin model files here (local dev)
└── Cargo.toml
```

---

## License

MIT

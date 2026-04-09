# ten-four

Push-to-talk speech-to-text for Linux. Press a hotkey → speak → release → text appears at your cursor. Works in terminals, browsers, code editors, anywhere.

- **Fully offline** — powered by [whisper.cpp](https://github.com/ggerganov/whisper.cpp) or [Vosk](https://alphacephei.com/vosk/)
- **Wayland + X11** — text injection via `ydotool` (Wayland-native) or `xdotool` (X11)
- **aarch64 ready** — tested on Raspberry Pi (aarch64 Debian)

---

## How It Works

```
ten-four daemon   ← run this in a terminal, keeps running
ten-four toggle   ← bind this to a hotkey in your DE
```

The daemon listens on a Unix socket (`/tmp/ten-four.sock`). Each call to `ten-four toggle` flips the daemon between **idle → recording → transcribing → idle**.

---

## 1. System Dependencies

### Debian / Ubuntu / Raspberry Pi OS

```bash
sudo apt update
sudo apt install -y \
  build-essential cmake libclang-dev \
  libasound2-dev \
  pipewire-pulse pulseaudio-utils \
  ydotool \
  pkg-config curl git wget unzip
```

> `pulseaudio-utils` includes both `pactl` (device selection) and `parec` (used internally when `--device` is set).

### Fedora

```bash
sudo dnf install -y \
  gcc cmake clang-devel \
  alsa-lib-devel pipewire-pulseaudio \
  ydotool \
  pkg-config curl git wget unzip
```

### Arch Linux / Manjaro

```bash
sudo pacman -S \
  base-devel cmake clang \
  alsa-lib pipewire-pulse \
  ydotool \
  pkg-config curl git wget unzip
```

---

## 2. Install Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

---

## 3. Install libvosk (required for Vosk engine)

The Vosk engine requires the native `libvosk` shared library. Download the prebuilt binary for your architecture:

**aarch64 (Raspberry Pi)**
```bash
wget https://github.com/alphacep/vosk-api/releases/download/v0.3.45/vosk-linux-aarch64-0.3.45.zip
unzip vosk-linux-aarch64-0.3.45.zip
sudo cp vosk-linux-aarch64-0.3.45/libvosk.so /usr/local/lib/
sudo ldconfig
```

**x86_64**
```bash
wget https://github.com/alphacep/vosk-api/releases/download/v0.3.45/vosk-linux-x86_64-0.3.45.zip
unzip vosk-linux-x86_64-0.3.45.zip
sudo cp vosk-linux-x86_64-0.3.45/libvosk.so /usr/local/lib/
sudo ldconfig
```

> `libvosk` is required at **build time** — the linker will fail without it. If you only want the Whisper engine, the easiest fix is to install libvosk anyway (it's just a `.so` file), or use feature flags if added in the future.

---

## 4. Build ten-four

```bash
git clone https://github.com/christiangantuangco/ten-four.git
cd ten-four
cargo build --release
```

Binary will be at `./target/release/ten-four`. Optionally install it system-wide:

```bash
sudo cp target/release/ten-four /usr/local/bin/ten-four
```

---

## 5. Download a Model

### Whisper models (default engine)

Auto-detection picks the **smallest** `.bin` file in `~/.local/share/ten-four/models/`.

```bash
mkdir -p ~/.local/share/ten-four/models
cd ~/.local/share/ten-four/models

# Recommended for low-power devices (Pi): tiny English-only model
wget https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin

# Or the multilingual base model (larger, slower, more accurate)
wget https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin
```

| Model       | Size  | Notes                              |
|-------------|-------|------------------------------------|
| tiny.en     | 75MB  | Fastest, English-only, good for Pi |
| base        | 142MB | Multilingual, slower on Pi         |
| small       | 466MB | Best accuracy, very slow on Pi     |

### Vosk models

```bash
mkdir -p ~/.local/share/ten-four/vosk-models
cd ~/.local/share/ten-four/vosk-models

# Small English model (~40MB, fast)
wget https://alphacephei.com/vosk/models/vosk-model-small-en-us-0.15.zip
unzip vosk-model-small-en-us-0.15.zip
```

More models at: https://alphacephei.com/vosk/models

---

## 6. Set Up ydotoold (Wayland)

`ydotool` requires a background daemon to inject keystrokes:

```bash
# Add yourself to the input group (once, then log out/in)
sudo usermod -aG input $USER

# Enable the ydotool systemd user service
systemctl --user enable --now ydotool
```

> If the service isn't available on your distro, run it manually: `ydotoold &`

### X11 users

```bash
sudo apt install xdotool   # or dnf/pacman equivalent
```

Pass `--injector xdotool` when starting the daemon.

---

## 7. Bind the Hotkey

Bind `ten-four toggle` to a key in your desktop environment.

### labwc (`~/.config/labwc/rc.xml`)

```xml
<?xml version="1.0"?>
<openbox_config xmlns="http://openbox.org/3.4/rc">
  <keyboard>
    <keybind key="C-grave">
      <action name="Execute">
        <command>/usr/local/bin/ten-four toggle</command>
      </action>
    </keybind>
  </keyboard>
</openbox_config>
```

Apply with: `labwc --reconfigure`

### GNOME

Settings → Keyboard → Custom Shortcuts → `+`
- **Command:** `ten-four toggle`
- **Shortcut:** your key combo

### KDE Plasma

System Settings → Shortcuts → Custom Shortcuts → New → Global Shortcut → Command/URL
- **Command:** `ten-four toggle`

### Sway / i3

```
bindsym $mod+grave exec ten-four toggle
```

### Hyprland

```
bind = CTRL, grave, exec, ten-four toggle
```

---

## 8. Run the Daemon

```bash
# Whisper engine (default, auto-picks smallest model)
./target/release/ten-four daemon

# Whisper with explicit device
./target/release/ten-four daemon --device "My Headset"

# Vosk engine
./target/release/ten-four daemon --engine vosk \
  --vosk-model ~/.local/share/ten-four/vosk-models/vosk-model-small-en-us-0.15 \
  --device "My Headset"

# List available microphones
./target/release/ten-four list-mics

# Test your hotkey without audio (confirms IPC is working)
./target/release/ten-four test-hotkey
```

---

## Configuration Reference

| Flag / Env | Description |
|---|---|
| `--engine whisper\|vosk` | Transcription engine (default: `whisper`) |
| `--model PATH` / `TEN_FOUR_MODEL` | Path to Whisper `.bin` model file |
| `--vosk-model PATH` / `VOSK_MODEL` | Path to Vosk model directory |
| `--device NAME` / `DEVICE` | Microphone source name (from `list-mics`) |
| `--injector ydotool\|xdotool` | Text injection method (default: `ydotool`) |
| `--socket PATH` | Unix socket path (default: `/tmp/ten-four.sock`) |
| `TEN_FOUR_USE_GPU=1` | Enable GPU inference for Whisper (requires CUDA/Vulkan build) |
| `YDOTOOL_SOCKET=PATH` | Override ydotoold socket path (default: `$XDG_RUNTIME_DIR/.ydotool_socket`) |
| `RUST_LOG=ten_four=debug` | Verbose logging |

---

## Commands

```
ten-four daemon        Start the daemon (run in a terminal or as a service)
ten-four toggle        Toggle recording (bind to a hotkey)
ten-four status        Print current state: idle / recording / transcribing
ten-four test-hotkey   Test hotkey IPC without audio
ten-four list-mics     List available microphone input devices
ten-four install-service  Print an example systemd user service file
```

---

## Troubleshooting

### "No input audio device found"
```bash
ten-four list-mics        # list all sources
pactl list sources        # raw PipeWire/PulseAudio sources
```

### "Could not connect to ten-four daemon"
The daemon isn't running. Start it manually in a terminal first.

### Text isn't appearing / ydotool errors
```bash
systemctl --user status ydotool
groups | grep input
# If not in input group: sudo usermod -aG input $USER  (then log out/in)
```

### Slow transcription on Raspberry Pi
Use `ggml-tiny.en.bin` (Whisper) or the small Vosk model. Both are designed for low-power devices. The daemon auto-detects the smallest Whisper model in the models directory.

### Vosk: "Failed to load model"
Ensure the path points to the **directory** (not a file inside it), and that `libvosk.so` is installed and `ldconfig` has been run.

---

## Project Structure

```
ten-four/
├── src/
│   ├── main.rs         # CLI: daemon | toggle | status | test-hotkey | list-mics
│   ├── daemon.rs       # State machine + task orchestration
│   ├── audio.rs        # cpal / parec recording + WAV conversion (16kHz mono)
│   ├── transcribe.rs   # TranscribeEngine trait, WhisperTranscriber, VoskTranscriber
│   ├── inject.rs       # ydotool / xdotool text injection
│   └── ipc.rs          # Unix socket client + server
└── Cargo.toml
```

---

## License

MIT

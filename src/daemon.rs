use crate::audio::{write_wav, AudioRecorder};
use crate::inject::Injector;
use crate::ipc::{self, Command};
use crate::transcribe::{transcribe_wav, TranscribeEngine};
use anyhow::Result;
use std::sync::{Arc, Mutex};
use tempfile::NamedTempFile;
use tokio::task;
use tracing::{error, info, warn};

#[derive(Debug, Clone, PartialEq)]
enum State {
    Idle,
    Recording,
    Transcribing,
}

impl State {
    fn as_str(&self) -> &'static str {
        match self {
            State::Idle => "idle",
            State::Recording => "recording",
            State::Transcribing => "transcribing",
        }
    }
}

pub async fn run(
    engine: Arc<dyn TranscribeEngine>,
    socket_path: String,
    injector_name: String,
    device: Option<String>,
) -> Result<()> {
    let injector = Injector::from_str(&injector_name);
    injector.check_available()?;

    if let Some(ref dev) = device {
        let status = std::process::Command::new("pactl")
            .args(["set-default-source", dev])
            .status();
        match status {
            Ok(s) if s.success() => info!("Default source set to: {}", dev),
            Ok(_) => anyhow::bail!("pactl failed to set source '{}'. Run `ten-four list-mics` to see valid names.", dev),
            Err(_) => anyhow::bail!("pactl not found — install pulseaudio-utils or pipewire-pulse to use --device"),
        }
    }

    let state: Arc<Mutex<State>> = Arc::new(Mutex::new(State::Idle));
    let stop_signal: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
    let injector = Arc::new(injector);
    let device: Arc<Option<String>> = Arc::new(device);

    let listener = ipc::bind_socket(&socket_path)?;
    info!("Listening on {}", socket_path);
    info!("Ready. Bind `ten-four toggle` to a hotkey in your desktop environment.");

    let socket_path_cleanup = socket_path.clone();
    ctrlc_handler(socket_path_cleanup);

    loop {
        let state_clone = Arc::clone(&state);
        let stop_clone = Arc::clone(&stop_signal);
        let engine_clone = Arc::clone(&engine);
        let injector_clone = Arc::clone(&injector);

        ipc::accept_one(&listener, |cmd| match cmd {
            Command::Status => state_clone.lock().unwrap().as_str().to_string(),
            Command::Toggle => {
                let current = state_clone.lock().unwrap().clone();
                match current {
                    State::Idle => {
                        *stop_clone.lock().unwrap() = false;
                        "start".to_string()
                    }
                    State::Recording => {
                        *stop_clone.lock().unwrap() = true;
                        "stop".to_string()
                    }
                    State::Transcribing => "busy".to_string(),
                }
            }
        })
        .await?;

        let current_state = state.lock().unwrap().clone();

        match current_state {
            State::Idle => {
                *state.lock().unwrap() = State::Recording;
                *stop_signal.lock().unwrap() = false;

                let state_for_task = Arc::clone(&state);
                let stop_for_task = Arc::clone(&stop_signal);
                let device_for_task = Arc::clone(&device);

                task::spawn(async move {
                    if let Err(e) = record_and_transcribe(
                        Arc::clone(&state_for_task),
                        stop_for_task,
                        engine_clone,
                        injector_clone,
                        device_for_task,
                    )
                    .await
                    {
                        error!("Error during record/transcribe: {:#}", e);
                        *state_for_task.lock().unwrap() = State::Idle;
                    }
                });
            }

            State::Recording => {
                info!("Stop signal sent to recording task");
            }

            State::Transcribing => {
                warn!("Received toggle while transcribing — ignoring");
            }
        }
    }
}

async fn record_and_transcribe(
    state: Arc<Mutex<State>>,
    stop_signal: Arc<Mutex<bool>>,
    engine: Arc<dyn TranscribeEngine>,
    injector: Arc<Injector>,
    device: Arc<Option<String>>,
) -> Result<()> {
    let device_name = (*device).clone();
    let recorder = AudioRecorder::new(device_name)?;
    let sample_rate = recorder.sample_rate();
    let channels = recorder.channels();

    let stop_for_recording = Arc::clone(&stop_signal);
    let samples = task::spawn_blocking(move || recorder.record_until_stop(stop_for_recording))
        .await??;

    *state.lock().unwrap() = State::Transcribing;
    info!("Transcribing...");

    let wav_file = NamedTempFile::new()?;
    let wav_path = wav_file.path().to_path_buf();
    write_wav(&wav_path, &samples, sample_rate, channels)?;

    let wav_path_clone = wav_path.clone();
    let text = task::spawn_blocking(move || transcribe_wav(engine.as_ref(), &wav_path_clone))
        .await??;

    if !text.is_empty() {
        info!("Injecting: {:?}", text);
        let text_clone = text.clone();
        task::spawn_blocking(move || injector.type_text(&text_clone)).await??;
    } else {
        info!("No speech detected");
    }

    *state.lock().unwrap() = State::Idle;
    info!("Ready");

    Ok(())
}

pub async fn run_test_hotkey(socket_path: String) -> Result<()> {
    let listener = ipc::bind_socket(&socket_path)?;
    info!("Test mode — listening on {}", socket_path);
    info!("Ready. Trigger your hotkey to test toggle.");

    let state: Arc<Mutex<State>> = Arc::new(Mutex::new(State::Idle));

    let socket_path_cleanup = socket_path.clone();
    ctrlc_handler(socket_path_cleanup);

    loop {
        let state_clone = Arc::clone(&state);
        ipc::accept_one(&listener, |cmd| match cmd {
            Command::Status => state_clone.lock().unwrap().as_str().to_string(),
            Command::Toggle => {
                let mut s = state_clone.lock().unwrap();
                match *s {
                    State::Idle => {
                        *s = State::Recording;
                        info!("[toggle] -> recording");
                        "start".to_string()
                    }
                    State::Recording => {
                        *s = State::Idle;
                        info!("[toggle] -> idle");
                        "stop".to_string()
                    }
                    State::Transcribing => "busy".to_string(),
                }
            }
        })
        .await?;
    }
}

fn ctrlc_handler(socket_path: String) {
    let _ = ctrlc::set_handler(move || {
        let _ = std::fs::remove_file(&socket_path);
        std::process::exit(0);
    });
}

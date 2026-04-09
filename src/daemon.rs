use crate::audio::{write_wav, AudioRecorder};
use crate::inject::Injector;
use crate::ipc::{self, Command};
use crate::transcribe::Transcriber;
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

pub async fn run(model_path: String, socket_path: String, injector_name: String) -> Result<()> {
    // Check injector is available before we start
    let injector = Injector::from_str(&injector_name);
    injector.check_available()?;

    // Load the Whisper model (blocking — happens once at startup)
    let transcriber = task::spawn_blocking(move || Transcriber::new(&model_path))
        .await??;
    let transcriber = Arc::new(transcriber);

    // Shared state
    let state: Arc<Mutex<State>> = Arc::new(Mutex::new(State::Idle));
    let stop_signal: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
    let injector = Arc::new(injector);

    // Bind the Unix socket
    let listener = ipc::bind_socket(&socket_path)?;
    info!("Listening on {}", socket_path);
    info!("Ready. Bind `ten-four toggle` to a hotkey in your desktop environment.");

    // Set up Ctrl+C handler to clean up the socket
    let socket_path_cleanup = socket_path.clone();
    ctrlc_handler(socket_path_cleanup);

    loop {
        let state_clone = Arc::clone(&state);
        let stop_clone = Arc::clone(&stop_signal);
        let transcriber_clone = Arc::clone(&transcriber);
        let injector_clone = Arc::clone(&injector);

        // Wait for a command from a client
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
                    State::Transcribing => {
                        "busy".to_string()
                    }
                }
            }
        })
        .await?;

        // Read what action was decided inside the handler
        let current_state = state.lock().unwrap().clone();

        match current_state {
            State::Idle => {
                // Transition to Recording and spawn the recording task
                *state.lock().unwrap() = State::Recording;
                *stop_signal.lock().unwrap() = false;

                let state_for_task = Arc::clone(&state);
                let stop_for_task = Arc::clone(&stop_signal);
                let transcriber_for_task = Arc::clone(&transcriber_clone);
                let injector_for_task = Arc::clone(&injector_clone);

                task::spawn(async move {
                    if let Err(e) = record_and_transcribe(
                        Arc::clone(&state_for_task),
                        stop_for_task,
                        transcriber_for_task,
                        injector_for_task,
                    )
                    .await
                    {
                        error!("Error during record/transcribe: {:#}", e);
                        *state_for_task.lock().unwrap() = State::Idle;
                    }
                });
            }

            State::Recording => {
                // stop_signal was already set to true inside the handler
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
    transcriber: Arc<Transcriber>,
    injector: Arc<Injector>,
) -> Result<()> {
    // Record audio in a blocking thread (cpal is sync)
    let recorder = AudioRecorder::new()?;
    let sample_rate = recorder.sample_rate();
    let channels = recorder.channels();

    let stop_for_recording = Arc::clone(&stop_signal);
    let samples = task::spawn_blocking(move || {
        recorder.record_until_stop(stop_for_recording)
    })
    .await??;

    // Transition to Transcribing
    *state.lock().unwrap() = State::Transcribing;
    info!("Transcribing...");

    // Write to a temp WAV file
    let wav_file = NamedTempFile::new()?;
    let wav_path = wav_file.path().to_path_buf();
    write_wav(&wav_path, &samples, sample_rate, channels)?;

    // Run Whisper inference in a blocking thread
    let wav_path_clone = wav_path.clone();
    let text = task::spawn_blocking(move || transcriber.transcribe_wav(&wav_path_clone))
        .await??;

    // Inject the text
    if !text.is_empty() {
        info!("Injecting: {:?}", text);
        let text_clone = text.clone();
        task::spawn_blocking(move || injector.type_text(&text_clone)).await??;
    } else {
        info!("No speech detected");
    }

    // Back to idle
    *state.lock().unwrap() = State::Idle;
    info!("Ready");

    Ok(())
}

fn ctrlc_handler(socket_path: String) {
    // Best-effort cleanup — remove the socket file on Ctrl+C
    let _ = ctrlc::set_handler(move || {
        let _ = std::fs::remove_file(&socket_path);
        std::process::exit(0);
    });
}

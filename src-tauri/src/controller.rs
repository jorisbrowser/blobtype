//! The recording state machine: Idle -> Recording -> Busy -> Idle.

use blobtype_core::audio::{
    meter_level, resample_linear, rms, to_pcm16, Recording, STT_SAMPLE_RATE,
};
use blobtype_core::config::{self, Config, Mode, Output};
use blobtype_core::live::{self, LiveEvent};
use blobtype_core::providers;
use blobtype_core::SttError;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};
use tokio::sync::{mpsc, oneshot};

use crate::capture::{AudioChunk, CaptureHandle};
use crate::overlay;

const MIN_RECORDING_SECS: f32 = 0.3;
const ERROR_TOAST: Duration = Duration::from_millis(3800);
/// How long "Inserted" stays up, so a transcript typed into the wrong window
/// is noticed while the "insert again" hint is still visible.
const DONE_TOAST: Duration = Duration::from_millis(2500);
const LEVEL_INTERVAL: Duration = Duration::from_millis(33);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stop {
    Finish,
    Cancel,
}

enum Phase {
    Idle,
    Recording(oneshot::Sender<Stop>),
    Busy,
}

pub struct Controller {
    phase: Mutex<Phase>,
    cancel_shortcut: Mutex<Option<Shortcut>>,
    /// The last transcript, kept in memory only (never written to disk).
    last_text: Mutex<Option<String>>,
    /// Bumped whenever something new takes over the blob, so a lingering
    /// toast does not hide it.
    generation: AtomicU64,
}

impl Controller {
    pub fn new() -> Self {
        Self {
            phase: Mutex::new(Phase::Idle),
            cancel_shortcut: Mutex::new(None),
            last_text: Mutex::new(None),
            generation: AtomicU64::new(0),
        }
    }

    fn next_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == generation
    }

    fn remember(&self, text: &str) {
        if !text.trim().is_empty() {
            *self.last_text.lock().unwrap() = Some(text.to_string());
        }
    }

    /// "Insert last" hotkey: type the previous transcript again. Always typed,
    /// never pasted, so the clipboard stays untouched.
    pub fn insert_last(&self, app: &AppHandle) {
        let mut phase = self.phase.lock().unwrap();
        if !matches!(*phase, Phase::Idle) {
            return;
        }
        let sounds = config::load_or_create(&config::config_path())
            .map(|c| c.sounds)
            .unwrap_or(true);
        let generation = self.next_generation();
        let app = app.clone();
        let Some(text) = self.last_text.lock().unwrap().clone() else {
            tauri::async_runtime::spawn(async move {
                show_toast(&app, "error", "Nothing to insert yet", sounds, generation).await;
            });
            return;
        };
        *phase = Phase::Busy;
        tauri::async_runtime::spawn(async move {
            let typed = tauri::async_runtime::spawn_blocking(move || {
                blobtype_input::wait_for_modifiers();
                blobtype_input::type_text(&text)
            })
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r);
            let ctl = app.state::<Controller>();
            ctl.set_idle(&app);
            match typed {
                Ok(()) => show_toast(&app, "done", "Inserted again", false, generation).await,
                Err(e) => show_toast(&app, "error", &e, sounds, generation).await,
            }
        });
    }

    /// Main hotkey: start, or stop-and-transcribe.
    pub fn toggle(&self, app: &AppHandle) {
        let mut phase = self.phase.lock().unwrap();
        match std::mem::replace(&mut *phase, Phase::Busy) {
            Phase::Idle => match self.start(app) {
                Ok(stop_tx) => *phase = Phase::Recording(stop_tx),
                Err(()) => *phase = Phase::Idle,
            },
            Phase::Recording(tx) => {
                let _ = tx.send(Stop::Finish);
                // stays Busy until the session task resets it
            }
            Phase::Busy => {}
        }
    }

    /// Cancel hotkey: abort without sending anything.
    pub fn cancel(&self) {
        let mut phase = self.phase.lock().unwrap();
        if matches!(&*phase, Phase::Recording(_)) {
            if let Phase::Recording(tx) = std::mem::replace(&mut *phase, Phase::Busy) {
                let _ = tx.send(Stop::Cancel);
            }
        }
    }

    pub fn is_idle(&self) -> bool {
        matches!(*self.phase.lock().unwrap(), Phase::Idle)
    }

    pub fn cancel_shortcut(&self) -> Option<Shortcut> {
        *self.cancel_shortcut.lock().unwrap()
    }

    fn set_idle(&self, app: &AppHandle) {
        self.unregister_cancel(app);
        *self.phase.lock().unwrap() = Phase::Idle;
        // A fullscreen app may have taken focus during the recording.
        crate::settings::update_fullscreen(app);
    }

    fn register_cancel(&self, app: &AppHandle, cfg: &Config) {
        let Ok(cancel) = cfg.cancel_hotkey.trim().parse::<Shortcut>() else {
            return;
        };
        if cfg.hotkey.trim().parse::<Shortcut>().ok() == Some(cancel) {
            return;
        }
        if app.global_shortcut().register(cancel).is_ok() {
            *self.cancel_shortcut.lock().unwrap() = Some(cancel);
        }
    }

    fn unregister_cancel(&self, app: &AppHandle) {
        if let Some(s) = self.cancel_shortcut.lock().unwrap().take() {
            let _ = app.global_shortcut().unregister(s);
        }
    }

    /// Returns the stop channel, or `Err(())` after showing an error toast.
    fn start(&self, app: &AppHandle) -> Result<oneshot::Sender<Stop>, ()> {
        let cfg = match config::load_or_create(&config::config_path()) {
            Ok(c) => c,
            Err(e) => {
                spawn_error(app.clone(), e.to_string(), false);
                return Err(());
            }
        };
        config::load_dotenv();

        // Resolve provider + key *before* recording so a missing key shows up immediately.
        let setup = prepare(&cfg);
        let setup = match setup {
            Ok(s) => s,
            Err(e) => {
                if matches!(e, SttError::MissingKey { .. }) {
                    crate::settings::open(app);
                }
                spawn_error(app.clone(), e.to_string(), cfg.sounds);
                return Err(());
            }
        };

        let (chunk_tx, chunk_rx) = mpsc::unbounded_channel::<AudioChunk>();
        let capture = match CaptureHandle::start(&cfg.microphone, chunk_tx) {
            Ok(c) => c,
            Err(e) => {
                spawn_error(app.clone(), e, cfg.sounds);
                return Err(());
            }
        };

        let generation = self.next_generation();
        overlay::show(app);
        overlay::emit_state_with_hints(
            app,
            "recording",
            None,
            cfg.sounds,
            vec![
                overlay::Hint::new(&cfg.hotkey, "done"),
                overlay::Hint::new(&cfg.cancel_hotkey, "cancel"),
            ],
        );
        self.register_cancel(app, &cfg);
        toggle_discord_mute(&cfg);

        let (stop_tx, stop_rx) = oneshot::channel();
        let app2 = app.clone();
        tauri::async_runtime::spawn(async move {
            session(app2, cfg, setup, capture, chunk_rx, stop_rx, generation).await;
        });
        Ok(stop_tx)
    }
}

enum Setup {
    Batch(Box<dyn providers::Provider>),
    Live,
}

fn prepare(cfg: &Config) -> Result<Setup, SttError> {
    match cfg.mode {
        Mode::Batch => {
            providers::create(&cfg.stt, cfg.language_code(), config::env_getter).map(Setup::Batch)
        }
        Mode::Live => {
            // Validate the key now; the session re-reads it when connecting.
            let p = cfg.live.provider.trim().to_ascii_lowercase();
            if !live::LIVE_PROVIDERS.contains(&p.as_str()) {
                return Err(SttError::Config(format!(
                    "Unknown live provider \"{}\"",
                    cfg.live.provider
                )));
            }
            if config::lookup_key(&p, "", config::env_getter).is_none() {
                return Err(SttError::MissingKey {
                    env: config::default_key_env(&p).to_string(),
                });
            }
            Ok(Setup::Live)
        }
    }
}

fn spawn_error(app: AppHandle, message: String, sound: bool) {
    tauri::async_runtime::spawn(async move {
        show_error(&app, message, sound).await;
    });
}

async fn show_error(app: &AppHandle, message: String, sound: bool) {
    log::warn!("{message}");
    overlay::show(app);
    overlay::emit_state(app, "error", Some(message), sound);
    tokio::time::sleep(ERROR_TOAST).await;
    overlay::hide(app).await;
}

/// A short toast that hides itself unless something newer took over the blob.
async fn show_toast(
    app: &AppHandle,
    state: &'static str,
    message: &str,
    sound: bool,
    generation: u64,
) {
    show_toast_with_hints(app, state, message, sound, generation, Vec::new()).await;
}

async fn show_toast_with_hints(
    app: &AppHandle,
    state: &'static str,
    message: &str,
    sound: bool,
    generation: u64,
    hints: Vec<overlay::Hint>,
) {
    if state == "error" {
        log::warn!("{message}");
    }
    overlay::show(app);
    overlay::emit_state_with_hints(app, state, Some(message.to_string()), sound, hints);
    tokio::time::sleep(if state == "error" {
        ERROR_TOAST
    } else {
        DONE_TOAST
    })
    .await;
    let ctl = app.state::<Controller>();
    overlay::hide_if(app, || ctl.is_current(generation)).await;
}

/// Press the key bound to "Toggle Mute" in Discord: once when recording
/// starts, once when it stops (also on cancel), so the two presses always pair up.
fn toggle_discord_mute(cfg: &Config) {
    if !cfg.discord.mute {
        return;
    }
    let Some(vk) = config::function_key_code(&cfg.discord.key) else {
        return;
    };
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(e) = blobtype_input::press_key(vk) {
            log::warn!("discord mute key: {e}");
        }
    });
}

fn type_out(output: Output, text: String) -> Result<(), String> {
    match output {
        Output::Type => blobtype_input::type_text(&text),
        Output::Paste => blobtype_input::paste_text(&text),
    }
}

async fn session(
    app: AppHandle,
    cfg: Config,
    setup: Setup,
    capture: CaptureHandle,
    mut chunks: mpsc::UnboundedReceiver<AudioChunk>,
    mut stop_rx: oneshot::Receiver<Stop>,
    generation: u64,
) {
    let ctl = app.state::<Controller>();
    let mut recording = Recording::new(cfg.max_recording_secs);
    let mut capped = false;

    // Live mode: audio goes to a websocket task, text comes back as events.
    let mut live_audio_tx: Option<mpsc::UnboundedSender<Vec<f32>>> = None;
    let mut live_task: Option<tauri::async_runtime::JoinHandle<Result<(), SttError>>> = None;
    let mut typing_task: Option<tauri::async_runtime::JoinHandle<(Option<String>, String)>> = None;
    if let Setup::Live = setup {
        let (atx, arx) = mpsc::unbounded_channel::<Vec<f32>>();
        let (etx, mut erx) = mpsc::unbounded_channel::<LiveEvent>();
        live_audio_tx = Some(atx);
        let cfg2 = cfg.clone();
        live_task = Some(tauri::async_runtime::spawn(async move {
            live::run(&cfg2, config::env_getter, arx, etx).await
        }));
        let output = cfg.output;
        typing_task = Some(tauri::async_runtime::spawn(async move {
            let mut first_error = None;
            let mut all = String::new();
            while let Some(LiveEvent::Text(piece)) = erx.recv().await {
                all.push_str(&piece);
                let r = tauri::async_runtime::spawn_blocking(move || type_out(output, piece)).await;
                if let Ok(Err(e)) = r {
                    first_error.get_or_insert(e);
                }
            }
            (first_error, all)
        }));
    }

    let mut last_level = Instant::now();
    let stop = loop {
        tokio::select! {
            s = &mut stop_rx => break s.unwrap_or(Stop::Cancel),
            chunk = chunks.recv() => {
                let Some(chunk) = chunk else { break Stop::Cancel };
                feed(&app, &cfg, &chunk, &mut recording, live_audio_tx.as_ref(), &mut last_level);
                if recording.is_full() {
                    capped = true;
                    break Stop::Finish;
                }
            }
        }
    };

    capture.stop();
    toggle_discord_mute(&cfg);
    // Flush what was captured between the stop request and the stream closing.
    while let Ok(chunk) = chunks.try_recv() {
        feed(
            &app,
            &cfg,
            &chunk,
            &mut recording,
            live_audio_tx.as_ref(),
            &mut last_level,
        );
    }
    ctl.unregister_cancel(&app);

    if stop == Stop::Cancel {
        if let Some(t) = &live_task {
            t.abort();
        }
        if let Some(t) = &typing_task {
            t.abort();
        }
        overlay::hide(&app).await;
        ctl.set_idle(&app);
        return;
    }

    let note = capped.then(|| format!("Max length ({} s) reached", cfg.max_recording_secs));
    overlay::emit_state(&app, "transcribing", note, cfg.sounds);

    let outcome: Result<Option<String>, String> = match setup {
        Setup::Batch(provider) => {
            if recording.duration_secs() < MIN_RECORDING_SECS {
                Err("Too short - nothing recorded".into())
            } else {
                match provider.transcribe(recording.to_wav()).await {
                    Err(e) => Err(e.to_string()),
                    Ok(text) if text.trim().is_empty() => Err("No speech detected".into()),
                    Ok(text) => {
                        // Before typing: if that fails, "insert last" can still recover it.
                        ctl.remember(&text);
                        let output = cfg.output;
                        match tauri::async_runtime::spawn_blocking(move || type_out(output, text))
                            .await
                        {
                            Ok(Ok(())) => Ok(None),
                            Ok(Err(e)) => Err(e),
                            Err(e) => Err(e.to_string()),
                        }
                    }
                }
            }
        }
        Setup::Live => {
            drop(live_audio_tx.take()); // closes the audio channel -> commit
            let live_result = match live_task.take() {
                Some(t) => t
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|r| r.map_err(|e| e.to_string())),
                None => Ok(()),
            };
            let (typing_err, text) = match typing_task.take() {
                Some(t) => t.await.unwrap_or_default(),
                None => Default::default(),
            };
            ctl.remember(&text);
            live_result.and_then(|_| typing_err.map_or(Ok(None), Err))
        }
    };

    match outcome {
        Ok(_) => {
            // Idle right away: a new recording may start while "Inserted" lingers.
            ctl.set_idle(&app);
            let hints = vec![overlay::Hint::new(&cfg.last_hotkey, "insert again")];
            show_toast_with_hints(&app, "done", "Inserted", false, generation, hints).await;
        }
        Err(msg) => {
            show_error(&app, msg, cfg.sounds).await;
            ctl.set_idle(&app);
        }
    }
}

fn feed(
    app: &AppHandle,
    cfg: &Config,
    chunk: &AudioChunk,
    recording: &mut Recording,
    live_tx: Option<&mpsc::UnboundedSender<Vec<f32>>>,
    last_level: &mut Instant,
) {
    if last_level.elapsed() >= LEVEL_INTERVAL {
        *last_level = Instant::now();
        overlay::emit_level(app, meter_level(rms(&chunk.samples)));
    }
    let mono16k = resample_linear(&chunk.samples, chunk.sample_rate, STT_SAMPLE_RATE);
    // Live mode also fills `recording` so the max-length cap applies there too.
    recording.push(&to_pcm16(&mono16k));
    if let (Mode::Live, Some(tx)) = (cfg.mode, live_tx) {
        let _ = tx.send(mono16k);
    }
}

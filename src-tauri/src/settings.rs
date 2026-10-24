//! The settings window. On first run it shows a short setup guide; afterwards
//! it is a regular settings page. Users never need to touch config.toml.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use blobtype_core::config::{self, Config};
use blobtype_core::settings::{self as core, ProviderInfo, UiSettings};
use blobtype_core::{audio, providers, SttError};
use serde::Serialize;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

pub const LABEL: &str = "settings";

/// The registered start/stop shortcut; changes take effect without a restart.
pub struct Hotkey(pub Mutex<Shortcut>);

/// Reasons to keep the start/stop and "insert last" shortcuts unregistered for a while.
pub struct HotkeyPause {
    /// `pause_in_fullscreen` from config.toml.
    enabled: bool,
    /// The settings window is recording a new shortcut.
    capturing: AtomicBool,
    /// A fullscreen app (most games) has focus and should get the keys itself.
    fullscreen: AtomicBool,
}

impl HotkeyPause {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            capturing: AtomicBool::new(false),
            fullscreen: AtomicBool::new(false),
        }
    }
}

/// The registered "insert last transcript" shortcut; `None` = off.
pub struct LastHotkey(pub Mutex<Option<Shortcut>>);

/// Empty = off.
pub fn parse_optional(wanted: &str) -> Result<Option<Shortcut>, String> {
    let wanted = wanted.trim();
    if wanted.is_empty() {
        return Ok(None);
    }
    wanted
        .parse()
        .map(Some)
        .map_err(|_| format!("\"{wanted}\" is not a valid shortcut."))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    settings: UiSettings,
    providers: Vec<ProviderInfo>,
    microphones: Vec<String>,
    autostart: bool,
    /// No usable key yet: show the setup guide.
    first_run: bool,
}

/// Open the window, or bring it to the front if it already exists.
pub fn open(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("settings.html".into()))
        .title("blobtype")
        .inner_size(580.0, 700.0)
        .min_inner_size(480.0, 560.0)
        .maximizable(false)
        .center()
        .build();
    if let Err(e) = built {
        log::error!("cannot open settings: {e}");
    }
}

fn load() -> Config {
    config::load_dotenv();
    config::load_or_create(&config::config_path()).unwrap_or_default()
}

/// Whether the next recording would find a key.
pub fn ready() -> bool {
    core::ready(&load())
}

#[tauri::command]
pub fn get_settings(app: AppHandle) -> State {
    let cfg = load();
    State {
        settings: UiSettings::from_config(&cfg),
        providers: core::providers(),
        microphones: crate::capture::input_devices(),
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        first_run: !core::ready(&cfg),
    }
}

/// `…abcd` of the stored key for `provider`, if there is one.
#[tauri::command]
pub fn key_hint(provider: String) -> Option<String> {
    let cfg = load();
    let env = core::key_env_for(&cfg, &provider);
    config::lookup_key(&provider, &env, config::env_getter).map(|k| core::key_hint(&k))
}

/// An empty `api_key` keeps the stored key.
#[tauri::command]
pub fn save_settings(
    app: AppHandle,
    settings: UiSettings,
    api_key: String,
    autostart: bool,
) -> Result<(), String> {
    let cfg = load();
    if settings.hotkey.trim() == settings.last_hotkey.trim() {
        return Err("The two shortcuts must be different.".into());
    }
    rebind(&app, &settings.hotkey)?;
    rebind_last(&app, &settings.last_hotkey)?;
    if !api_key.trim().is_empty() {
        let env = core::key_env_for(&cfg, &settings.provider);
        core::save_key(&env, &api_key).map_err(|e| e.to_string())?;
    }
    core::save_config(&config::config_path(), &settings).map_err(|e| e.to_string())?;
    let launch = app.autolaunch();
    if launch.is_enabled().unwrap_or(false) != autostart {
        let r = if autostart {
            launch.enable()
        } else {
            launch.disable()
        };
        r.map_err(|e| format!("Could not change \"Start with Windows\": {e}"))?;
    }
    crate::tray::sync_autostart(&app);
    Ok(())
}

/// Send one second of near-silence to the provider. An empty `api_key` tests
/// the stored key.
#[tauri::command]
pub async fn test_key(
    provider: String,
    api_key: String,
    model: String,
    base_url: String,
) -> Result<String, String> {
    let cfg = load();
    let env = core::key_env_for(&cfg, &provider);
    let stt = config::SttConfig {
        provider: provider.clone(),
        model,
        base_url,
        api_key_env: if provider == cfg.stt.provider {
            cfg.stt.api_key_env.clone()
        } else {
            String::new()
        },
    };
    let typed = api_key.trim().to_string();
    let getter = move |name: &str| {
        if !typed.is_empty() && name == env {
            Some(typed.clone())
        } else {
            config::env_getter(name)
        }
    };
    let p = providers::create(&stt, None, getter).map_err(|e| match e {
        SttError::MissingKey { .. } => "Paste your API key first.".to_string(),
        e => e.to_string(),
    })?;
    // Faint noise rather than digital zero: some APIs reject all-zero audio.
    let samples: Vec<i16> = (0..16_000).map(|i| ((i * 7919) % 41) as i16 - 20).collect();
    match p.transcribe(audio::encode_wav(&samples, 16_000)).await {
        Ok(_) => Ok("Connected. Your key works.".into()),
        Err(SttError::Api {
            status: 401 | 403, ..
        }) => Err("The service rejected this key. Copy it again and check the provider.".into()),
        Err(SttError::Network(_)) => {
            Err("Could not reach the service. Check your internet connection.".into())
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Open the provider's "create API key" page in the browser.
#[tauri::command]
pub fn open_key_page(provider: String) {
    if let Some(p) = core::provider(&provider).filter(|p| p.key_url.starts_with("https://")) {
        crate::tray::open_target(p.key_url);
    }
}

/// While the user records a new shortcut, the current ones must not fire.
#[tauri::command]
pub fn suspend_hotkey(app: AppHandle, suspended: bool) {
    app.state::<HotkeyPause>()
        .capturing
        .store(suspended, Ordering::Relaxed);
    sync_hotkey(&app);
}

/// Register or unregister the shortcuts according to `HotkeyPause`. Call on the
/// main thread: the shortcut handler runs there and locks `Hotkey` too.
fn sync_hotkey(app: &AppHandle) {
    let pause = app.state::<HotkeyPause>();
    let paused =
        pause.capturing.load(Ordering::Relaxed) || pause.fullscreen.load(Ordering::Relaxed);
    let current = *app.state::<Hotkey>().0.lock().unwrap();
    let last = *app.state::<LastHotkey>().0.lock().unwrap();
    let gs = app.global_shortcut();
    for s in std::iter::once(current).chain(last) {
        if paused {
            let _ = gs.unregister(s);
        } else if !gs.is_registered(s) {
            let _ = gs.register(s);
        }
    }
}

/// Re-check whether a fullscreen app has focus and (un)register the shortcut
/// so that e.g. a game receives Ctrl+Space instead of blobtype. Returns whether
/// the shortcut is now paused for fullscreen.
pub fn update_fullscreen(app: &AppHandle) -> bool {
    let pause = app.state::<HotkeyPause>();
    // A running recording must stay stoppable.
    let fullscreen = pause.enabled
        && app.state::<crate::controller::Controller>().is_idle()
        && blobtype_input::foreground_is_fullscreen();
    if pause.fullscreen.swap(fullscreen, Ordering::Relaxed) != fullscreen {
        let h = app.clone();
        let _ = app.run_on_main_thread(move || sync_hotkey(&h));
    }
    fullscreen
}

/// Re-check on every foreground window change; Windows notifies us, no polling.
pub fn watch_fullscreen(app: AppHandle) {
    if app.state::<HotkeyPause>().enabled {
        blobtype_input::on_foreground_change(move || {
            update_fullscreen(&app);
        });
    }
}

/// Press the Discord mute key after a short delay, so Discord can record it
/// as the "Toggle Mute" keybind (F13-F24 cannot be pressed by hand).
#[tauri::command]
pub async fn send_discord_key(key: String) -> Result<(), String> {
    let vk =
        config::function_key_code(&key).ok_or_else(|| format!("\"{key}\" is not an F-key."))?;
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    tauri::async_runtime::spawn_blocking(move || blobtype_input::press_key(vk))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn open_config_file() {
    let path = config::config_path();
    let _ = config::load_or_create(&path);
    crate::tray::open_target(&path);
}

#[tauri::command]
pub fn close_settings(app: AppHandle) {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.close();
    }
}

/// Swap the global shortcut. Leaves the old one active on failure.
fn rebind(app: &AppHandle, wanted: &str) -> Result<(), String> {
    let new: Shortcut = wanted
        .trim()
        .parse()
        .map_err(|_| format!("\"{wanted}\" is not a valid shortcut."))?;
    let state = app.state::<Hotkey>();
    let mut current = state.0.lock().unwrap();
    let gs = app.global_shortcut();
    if *current == new {
        if !gs.is_registered(new) {
            let _ = gs.register(new);
        }
        return Ok(());
    }
    gs.register(new).map_err(|_| {
        "That shortcut is already used by another app. Try a different one.".to_string()
    })?;
    let _ = gs.unregister(*current);
    *current = new;
    Ok(())
}

/// Swap (or turn off) the "insert last" shortcut. Leaves the old one active on failure.
fn rebind_last(app: &AppHandle, wanted: &str) -> Result<(), String> {
    let new = parse_optional(wanted)?;
    let state = app.state::<LastHotkey>();
    let mut current = state.0.lock().unwrap();
    let gs = app.global_shortcut();
    if *current == new {
        if let Some(s) = new.filter(|s| !gs.is_registered(*s)) {
            let _ = gs.register(s);
        }
        return Ok(());
    }
    if let Some(s) = new {
        gs.register(s).map_err(|_| {
            "The \"insert last\" shortcut is already used by another app. Try a different one."
                .to_string()
        })?;
    }
    if let Some(old) = *current {
        let _ = gs.unregister(old);
    }
    *current = new;
    Ok(())
}

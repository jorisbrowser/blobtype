//! The blob window: transparent, click-through, always on top, never focused.

use serde::Serialize;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition};

pub const STATE_EVENT: &str = "blob-state";
pub const LEVEL_EVENT: &str = "blob-level";
const FADE_OUT: Duration = Duration::from_millis(450);
const TOPMOST_REASSERT: Duration = Duration::from_millis(1500);

/// A shortcut shown next to the blob, e.g. `Esc` "cancel".
#[derive(Serialize, Clone)]
pub struct Hint {
    pub keys: String,
    pub label: &'static str,
}

impl Hint {
    pub fn new(keys: &str, label: &'static str) -> Self {
        Self {
            keys: keys.trim().to_string(),
            label,
        }
    }
}

#[derive(Serialize, Clone)]
pub struct StatePayload {
    /// "recording" | "transcribing" | "done" | "error" | "hidden"
    pub state: &'static str,
    pub message: Option<String>,
    pub sound: bool,
    pub hints: Vec<Hint>,
}

pub fn emit_state(app: &AppHandle, state: &'static str, message: Option<String>, sound: bool) {
    emit_state_with_hints(app, state, message, sound, Vec::new());
}

pub fn emit_state_with_hints(
    app: &AppHandle,
    state: &'static str,
    message: Option<String>,
    sound: bool,
    hints: Vec<Hint>,
) {
    let hints = hints.into_iter().filter(|h| !h.keys.is_empty()).collect();
    let _ = app.emit(
        STATE_EVENT,
        StatePayload {
            state,
            message,
            sound,
            hints,
        },
    );
}

pub fn emit_level(app: &AppHandle, level: f32) {
    let _ = app.emit(LEVEL_EVENT, level);
}

pub fn init(app: &AppHandle) -> Result<(), String> {
    let w = app
        .get_webview_window("overlay")
        .ok_or("overlay window missing")?;
    w.set_ignore_cursor_events(true)
        .map_err(|e| e.to_string())?;
    let _ = w.set_focusable(false);
    Ok(())
}

/// Bottom-center of the primary work area (above the taskbar), then show.
pub fn show(app: &AppHandle) {
    let Some(w) = app.get_webview_window("overlay") else {
        return;
    };
    if let Ok(size) = w.outer_size() {
        let area = blobtype_input::primary_work_area().or_else(|| {
            let m = w.primary_monitor().ok().flatten()?;
            let (p, s) = (m.position(), m.size());
            Some((p.x, p.y, p.x + s.width as i32, p.y + s.height as i32))
        });
        if let Some((l, _t, r, b)) = area {
            let x = l + ((r - l) - size.width as i32) / 2;
            let y = b - size.height as i32;
            let _ = w.set_position(PhysicalPosition::new(x, y));
        }
    }
    let _ = w.set_always_on_top(true);
    let _ = w.show();
}

/// Fade out in the page, then hide the window.
pub async fn hide(app: &AppHandle) {
    hide_if(app, || true).await;
}

/// Like `hide`, but does nothing once `current()` turns false (a new recording
/// took over the blob in the meantime).
pub async fn hide_if(app: &AppHandle, current: impl Fn() -> bool) {
    if !current() {
        return;
    }
    emit_state(app, "hidden", None, false);
    tokio::time::sleep(FADE_OUT).await;
    if !current() {
        return;
    }
    if let Some(w) = app.get_webview_window("overlay") {
        let _ = w.hide();
    }
}

/// Other topmost windows can leapfrog us; re-assert while visible.
pub fn spawn_topmost_guard(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(TOPMOST_REASSERT).await;
            if let Some(w) = app.get_webview_window("overlay") {
                if w.is_visible().unwrap_or(false) {
                    let _ = w.set_always_on_top(true);
                }
            }
        }
    });
}

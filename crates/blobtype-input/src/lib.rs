//! Text injection into whatever window currently has keyboard focus.
//!
//! Uses only documented user-mode Win32 APIs (`SendInput`, clipboard); it never
//! injects into or hooks other processes.

mod plan;

pub use plan::{plan_keys, KeyAction};

#[cfg(windows)]
mod windows_impl;
#[cfg(windows)]
pub use windows_impl::{paste_text, press_key, type_text, wait_for_modifiers};

#[cfg(not(windows))]
pub fn type_text(_text: &str) -> Result<(), String> {
    Err("text injection is only supported on Windows".into())
}

#[cfg(not(windows))]
pub fn paste_text(_text: &str) -> Result<(), String> {
    Err("text injection is only supported on Windows".into())
}

#[cfg(not(windows))]
pub fn wait_for_modifiers() {}

#[cfg(not(windows))]
pub fn press_key(_vk: u16) -> Result<(), String> {
    Err("key injection is only supported on Windows".into())
}

/// Work area (screen minus taskbar) of the primary monitor in physical pixels:
/// `(left, top, right, bottom)`.
#[cfg(windows)]
pub use windows_impl::primary_work_area;

/// Whether the foreground window belongs to another process and covers its
/// whole monitor (exclusive or borderless fullscreen, as games use).
#[cfg(windows)]
pub use windows_impl::foreground_is_fullscreen;

#[cfg(not(windows))]
pub fn foreground_is_fullscreen() -> bool {
    false
}

/// Call `f` (on a dedicated thread) whenever another window comes to the
/// foreground. Event-driven via `SetWinEventHook`; nothing runs in between.
#[cfg(windows)]
pub use windows_impl::on_foreground_change;

#[cfg(not(windows))]
pub fn on_foreground_change(_f: impl Fn() + Send + Sync + 'static) {}

#[cfg(not(windows))]
pub fn primary_work_area() -> Option<(i32, i32, i32, i32)> {
    None
}

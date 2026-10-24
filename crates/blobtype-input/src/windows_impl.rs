use std::mem::size_of;
use std::thread::sleep;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{GlobalFree, HANDLE};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows_sys::Win32::System::Ole::CF_UNICODETEXT;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, VK_CONTROL, VK_LWIN, VK_MENU, VK_RETURN, VK_RWIN, VK_SHIFT, VK_TAB,
};

use crate::plan::{plan_keys, KeyAction};

/// Key events per `SendInput` call, and the pause between calls. Small batches
/// keep slow targets (terminals, remote desktops, browsers) from dropping keys.
const CHUNK_KEYS: usize = 24;
const CHUNK_PAUSE: Duration = Duration::from_millis(4);
const VK_V: u16 = 0x56;

fn key_input(vk: u16, scan: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send(inputs: &[INPUT]) -> Result<(), String> {
    if inputs.is_empty() {
        return Ok(());
    }
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
    if sent as usize != inputs.len() {
        // Typically UIPI: the focused window belongs to an elevated process.
        return Err(format!(
            "SendInput delivered {sent}/{} events (is the target app running as administrator?)",
            inputs.len()
        ));
    }
    Ok(())
}

fn action_inputs(a: KeyAction) -> [INPUT; 2] {
    match a {
        KeyAction::Unicode(u) => [
            key_input(0, u, KEYEVENTF_UNICODE),
            key_input(0, u, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP),
        ],
        KeyAction::Enter => [
            key_input(VK_RETURN, 0, 0),
            key_input(VK_RETURN, 0, KEYEVENTF_KEYUP),
        ],
        KeyAction::Tab => [
            key_input(VK_TAB, 0, 0),
            key_input(VK_TAB, 0, KEYEVENTF_KEYUP),
        ],
    }
}

/// Type `text` into the focused window with simulated Unicode keystrokes.
pub fn type_text(text: &str) -> Result<(), String> {
    let actions = plan_keys(text);
    for chunk in actions.chunks(CHUNK_KEYS) {
        let inputs: Vec<INPUT> = chunk.iter().flat_map(|a| action_inputs(*a)).collect();
        send(&inputs)?;
        sleep(CHUNK_PAUSE);
    }
    Ok(())
}

/// How long `wait_for_modifiers` waits for the user to let go of Ctrl/Alt/Shift/Win.
const MODIFIER_WAIT: Duration = Duration::from_secs(1);

fn modifiers_held() -> bool {
    [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN]
        .iter()
        .any(|&vk| unsafe { GetAsyncKeyState(vk as i32) } < 0)
}

/// Block (up to a second) until no modifier is held, so injected keys are not
/// combined with the ones the user still holds from a hotkey.
pub fn wait_for_modifiers() {
    let start = Instant::now();
    while modifiers_held() && start.elapsed() < MODIFIER_WAIT {
        sleep(Duration::from_millis(10));
    }
}

/// Press and release the virtual key `vk`, after the modifiers are released
/// (a listener sees the bare key and not e.g. Ctrl+F24).
pub fn press_key(vk: u16) -> Result<(), String> {
    wait_for_modifiers();
    send(&[key_input(vk, 0, 0), key_input(vk, 0, KEYEVENTF_KEYUP)])
}

struct Clipboard;

impl Clipboard {
    fn open() -> Result<Self, String> {
        for _ in 0..10 {
            if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                return Ok(Clipboard);
            }
            sleep(Duration::from_millis(10));
        }
        Err("clipboard is busy".into())
    }
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        unsafe { CloseClipboard() };
    }
}

fn read_clipboard_text() -> Option<String> {
    let _c = Clipboard::open().ok()?;
    unsafe {
        let h: HANDLE = GetClipboardData(CF_UNICODETEXT as u32);
        if h.is_null() {
            return None;
        }
        let p = GlobalLock(h) as *const u16;
        if p.is_null() {
            return None;
        }
        let mut len = 0;
        while *p.add(len) != 0 {
            len += 1;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
        GlobalUnlock(h);
        Some(s)
    }
}

fn write_clipboard_text(text: &str) -> Result<(), String> {
    let _c = Clipboard::open()?;
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        EmptyClipboard();
        let mem = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
        if mem.is_null() {
            return Err("out of memory".into());
        }
        let p = GlobalLock(mem) as *mut u16;
        if p.is_null() {
            GlobalFree(mem);
            return Err("cannot lock clipboard memory".into());
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
        GlobalUnlock(mem);
        if SetClipboardData(CF_UNICODETEXT as u32, mem).is_null() {
            GlobalFree(mem);
            return Err("cannot set clipboard".into());
        }
    }
    Ok(())
}

/// Put `text` on the clipboard, press Ctrl+V, then restore the previous text
/// clipboard content (non-text content is not preserved).
pub fn paste_text(text: &str) -> Result<(), String> {
    let previous = read_clipboard_text();
    write_clipboard_text(text)?;
    send(&[
        key_input(VK_CONTROL, 0, 0),
        key_input(VK_V, 0, 0),
        key_input(VK_V, 0, KEYEVENTF_KEYUP),
        key_input(VK_CONTROL, 0, KEYEVENTF_KEYUP),
    ])?;
    // Give the target time to read the clipboard before we put the old text back.
    sleep(Duration::from_millis(250));
    if let Some(prev) = previous {
        let _ = write_clipboard_text(&prev);
    }
    Ok(())
}

pub fn primary_work_area() -> Option<(i32, i32, i32, i32)> {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETWORKAREA};
    let mut r = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    let ok = unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut r as *mut RECT as *mut _, 0) };
    (ok != 0).then_some((r.left, r.top, r.right, r.bottom))
}

pub fn foreground_is_fullscreen() -> bool {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONULL,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetDesktopWindow, GetForegroundWindow, GetShellWindow, GetWindowRect,
        GetWindowThreadProcessId,
    };
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() || hwnd == GetDesktopWindow() || hwnd == GetShellWindow() {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == GetCurrentProcessId() {
            return false;
        }
        // The desktop behind the icons also spans the whole monitor.
        let mut class = [0u16; 16];
        let n = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32).max(0) as usize;
        let class = String::from_utf16_lossy(&class[..n]);
        if class == "Progman" || class == "WorkerW" {
            return false;
        }
        let mut r: RECT = std::mem::zeroed();
        if GetWindowRect(hwnd, &mut r) == 0 {
            return false;
        }
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL);
        if monitor.is_null() {
            return false;
        }
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return false;
        }
        let m = info.rcMonitor;
        r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom
    }
}

static FOREGROUND_CALLBACK: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> =
    std::sync::OnceLock::new();

pub fn on_foreground_change(f: impl Fn() + Send + Sync + 'static) {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, TranslateMessage, EVENT_SYSTEM_FOREGROUND, MSG,
        WINEVENT_OUTOFCONTEXT,
    };

    unsafe extern "system" fn hook(
        _: HWINEVENTHOOK,
        _: u32,
        _: HWND,
        _: i32,
        _: i32,
        _: u32,
        _: u32,
    ) {
        if let Some(f) = FOREGROUND_CALLBACK.get() {
            f();
        }
    }

    if FOREGROUND_CALLBACK.set(Box::new(f)).is_err() {
        return;
    }
    // Out-of-context events arrive through this thread's message queue.
    std::thread::spawn(|| unsafe {
        let h = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            std::ptr::null_mut(),
            Some(hook),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );
        if h.is_null() {
            return;
        }
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
}

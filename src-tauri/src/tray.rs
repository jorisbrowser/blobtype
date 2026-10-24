use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_autostart::ManagerExt;

/// Open a file or URL with the system default handler.
pub fn open_target(target: impl AsRef<std::ffi::OsStr>) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", ""])
            .arg(target)
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(target).spawn();
    }
}

struct AutostartItem(CheckMenuItem<Wry>);

/// Reflect the real autostart state in the tray menu (it can change from the settings window).
pub fn sync_autostart(app: &AppHandle) {
    if let Some(item) = app.try_state::<AutostartItem>() {
        let _ = item
            .0
            .set_checked(app.autolaunch().is_enabled().unwrap_or(false));
    }
}

pub fn setup(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let autostart_on = app.autolaunch().is_enabled().unwrap_or(false);
    let autostart = CheckMenuItem::with_id(
        app,
        "autostart",
        "Start with Windows",
        true,
        autostart_on,
        None::<&str>,
    )?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit blobtype", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &settings,
            &autostart,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    app.manage(AutostartItem(autostart.clone()));

    let mut builder = TrayIconBuilder::new()
        .menu(&menu)
        .tooltip("blobtype: click for settings")
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                crate::settings::open(tray.app_handle());
            }
        })
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "autostart" => {
                let launch = app.autolaunch();
                let result = if launch.is_enabled().unwrap_or(false) {
                    launch.disable()
                } else {
                    launch.enable()
                };
                if let Err(e) = result {
                    log::error!("autostart toggle failed: {e}");
                }
                sync_autostart(app);
            }
            "settings" => crate::settings::open(app),
            "quit" => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

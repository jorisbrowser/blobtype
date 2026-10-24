mod capture;
mod controller;
mod overlay;
mod settings;
mod tray;

use blobtype_core::config::{self, Config};
use controller::Controller;
use tauri::Manager;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

pub fn run() {
    config::load_dotenv();
    // A broken config must not prevent startup: fall back to defaults here and
    // report the error as a toast the first time the hotkey is pressed.
    let cfg = config::load_or_create(&config::config_path()).unwrap_or_else(|e| {
        log::error!("{e}");
        Config::default()
    });
    let hotkey: Shortcut = cfg.hotkey.trim().parse().unwrap_or_else(|_| {
        log::error!("invalid hotkey \"{}\", using Ctrl+Space", cfg.hotkey);
        "Ctrl+Space".parse().expect("static hotkey")
    });
    let last_hotkey = settings::parse_optional(&cfg.last_hotkey)
        .unwrap_or_else(|e| {
            log::error!("{e}");
            None
        })
        .filter(|s| *s != hotkey);

    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .build(),
        )
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            settings::open(app)
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, shortcut, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    let ctl = app.state::<Controller>();
                    let current = *app.state::<settings::Hotkey>().0.lock().unwrap();
                    let last = *app.state::<settings::LastHotkey>().0.lock().unwrap();
                    if *shortcut == current {
                        // Catches apps that went fullscreen without a focus
                        // change; the shortcut is released from now on.
                        if !settings::update_fullscreen(app) {
                            ctl.toggle(app);
                        }
                    } else if Some(*shortcut) == ctl.cancel_shortcut() {
                        ctl.cancel();
                    } else if Some(*shortcut) == last && !settings::update_fullscreen(app) {
                        ctl.insert_last(app);
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            settings::get_settings,
            settings::key_hint,
            settings::save_settings,
            settings::test_key,
            settings::open_key_page,
            settings::suspend_hotkey,
            settings::send_discord_key,
            settings::open_config_file,
            settings::close_settings,
        ])
        .manage(Controller::new())
        .manage(settings::Hotkey(std::sync::Mutex::new(hotkey)))
        .manage(settings::HotkeyPause::new(cfg.pause_in_fullscreen))
        .manage(settings::LastHotkey(std::sync::Mutex::new(last_hotkey)))
        .setup(move |app| {
            let handle = app.handle().clone();
            overlay::init(&handle)?;
            overlay::spawn_topmost_guard(handle.clone());
            tray::setup(&handle)?;
            if let Err(e) = handle.global_shortcut().register(hotkey) {
                // Most likely another app owns the hotkey.
                log::error!("cannot register hotkey: {e}");
            }
            settings::watch_fullscreen(handle.clone());
            if let Some(last) = last_hotkey {
                if let Err(e) = handle.global_shortcut().register(last) {
                    log::error!("cannot register \"insert last\" hotkey: {e}");
                }
            }
            // First run (or no key yet): open the setup guide right away.
            if !settings::ready() {
                settings::open(&handle);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running blobtype");
}

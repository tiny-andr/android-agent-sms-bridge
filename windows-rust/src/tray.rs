//! Tray icon and its menu — ported from `buildTray` / `refreshTrayMenu` in
//! `windows/main.js`, including the exact labels (translated through the same
//! dictionary) and the "sync now" / autostart / quit actions.
//!
//! The whole menu is rebuilt on every refresh, like the Electron version: the
//! checked state and the phone list are snapshots by design.

use serde_json::Value;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::app::{self, AppState};
use crate::i18n::t;

const TRAY_ID: &str = "smsbridge-tray";

fn tray_image() -> Option<tauri::image::Image<'static>> {
    // An .ico carries the sizes Windows asks for (16/20/24/32) and stays crisp
    // at any DPI; the PNG is the fallback, the app icon the last resort, so the
    // tray never ends up as an empty square.
    for bytes in [
        include_bytes!("../assets/tray.ico").as_slice(),
        include_bytes!("../assets/tray.png").as_slice(),
        include_bytes!("../assets/icon.png").as_slice(),
    ] {
        if let Ok(image) = tauri::image::Image::from_bytes(bytes) {
            return Some(image);
        }
    }
    None
}

fn build_menu(
    app: &AppHandle,
    phones: &[Value],
    autostart: bool,
    running: bool,
) -> tauri::Result<Menu<Wry>> {
    let menu = Menu::new(app)?;
    let title = MenuItem::with_id(app, "title", app::APP_NAME, false, None::<&str>)?;
    let paired_count = phones.len().to_string();
    let paired = MenuItem::with_id(
        app,
        "paired",
        t("已配对手机 · {n}", &[("n", &paired_count)]),
        false,
        None::<&str>,
    )?;
    let sync = MenuItem::with_id(
        app,
        "sync",
        if running {
            t("同步中…", &[])
        } else {
            t("立即同步全部", &[])
        },
        !running,
        None::<&str>,
    )?;
    let open = MenuItem::with_id(app, "open", t("打开主界面", &[]), true, None::<&str>)?;
    let autostart_item = CheckMenuItem::with_id(
        app,
        "autostart",
        t("开机自启", &[]),
        true,
        autostart,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", t("退出", &[]), true, None::<&str>)?;

    menu.append(&title)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&paired)?;
    for phone in phones.iter().take(6) {
        let id = phone.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let name = phone.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let url = phone
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .to_string();
        let item = MenuItem::with_id(
            app,
            format!("phone:{id}"),
            format!("  {name}  ·  {url}"),
            true,
            None::<&str>,
        )?;
        menu.append(&item)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&sync)?;
    menu.append(&open)?;
    menu.append(&autostart_item)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&quit)?;
    Ok(menu)
}

fn tooltip(app: &AppHandle, count: usize) -> String {
    let synced = if app::last_sync_at(app) != 0 {
        t(" · 已同步", &[])
    } else {
        String::new()
    };
    let count_text = count.to_string();
    format!(
        "{} · {}{}",
        app::APP_NAME,
        t("{n} 台手机", &[("n", &count_text)]),
        synced
    )
}

fn on_menu_event(app: &AppHandle, id: &str) {
    match id {
        "sync" => app::sync_all_from_tray(app),
        "open" => app::show_window(app),
        "autostart" => {
            let current = {
                let state = app.state::<AppState>();
                let store = state.store.lock().unwrap();
                store.setting_bool("autoStart", false)
            };
            app::set_autostart_from_tray(app, !current);
        }
        "quit" => app::quit(app),
        other => {
            if let Some(phone_id) = other.strip_prefix("phone:") {
                app::show_window(app);
                app::sync_device_from_tray(app, phone_id.to_string());
            }
        }
    }
}

/// Create the tray on first call, rebuild its menu afterwards.
pub fn refresh(app: &AppHandle) {
    let (phones, autostart) = {
        let state = app.state::<AppState>();
        let store = state.store.lock().unwrap();
        (store.list_phones(), store.setting_bool("autoStart", false))
    };
    let running = app::sync_running(app);
    let text = tooltip(app, phones.len());

    let menu = match build_menu(app, &phones, autostart, running) {
        Ok(menu) => menu,
        Err(err) => {
            eprintln!("[tray] menu build failed: {err}");
            return;
        }
    };

    match app.tray_by_id(TRAY_ID) {
        Some(tray) => {
            if let Err(err) = tray.set_menu(Some(menu)) {
                eprintln!("[tray] set_menu failed: {err}");
            }
            let _ = tray.set_tooltip(Some(text));
        }
        None => {
            let mut builder = TrayIconBuilder::with_id(TRAY_ID)
                .tooltip(text)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| on_menu_event(app, event.id().as_ref()))
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        app::show_window(tray.app_handle());
                    }
                });
            if let Some(image) = tray_image() {
                builder = builder.icon(image);
            }
            match builder.build(app) {
                Ok(tray) => {
                    if let Err(err) = tray.set_menu(Some(menu)) {
                        eprintln!("[tray] set_menu failed: {err}");
                    }
                }
                Err(err) => eprintln!("[tray] create failed: {err}"),
            }
        }
    }
}

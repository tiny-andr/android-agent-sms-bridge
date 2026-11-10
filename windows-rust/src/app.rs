//! Application shell: window / tray / scheduler / sync orchestration.
//!
//! This is the Rust equivalent of the Electron main process (`windows/main.js`):
//! same boot order, same IPC channels (registered in `commands.rs`), same event
//! names, same data directory and file formats. Everything user-visible is meant
//! to behave exactly like the 2.1.0 Electron build.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};
use tauri::window::Color;
use tauri::webview::PageLoadEvent;
use tauri::{
    AppHandle, Emitter, Manager, RunEvent, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
    WindowEvent,
};

use crate::i18n::{self, t};
use crate::migrate;
use crate::smsbridge::BridgeError;
use crate::store::{now_ms, Store};
use crate::sync::{sync_phone, SyncOptions, SyncOutcome, SyncProgress};
use crate::updater::UpdateAsset;

pub const APP_NAME: &str = "Windows SMS Bridge";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
/// `snapshot()` sends at most this many cached messages to the renderer.
const MAX_SNAPSHOT_MESSAGES: usize = 20000;

// --------------------------------------------------------------------- state

#[derive(Default)]
pub struct SyncState {
    pub running: bool,
    pub last_at: i64,
    pub last_summary: Option<Value>,
    pub errors: Vec<Value>,
}

#[derive(Default)]
pub struct UpdateState {
    pub available: bool,
    pub version: String,
    pub notes: String,
    pub checking: bool,
    pub progress: Option<(u64, u64)>,
    pub error: String,
}

pub struct AppState {
    pub data_dir: PathBuf,
    pub store: Mutex<Store>,
    pub quitting: AtomicBool,
    pub maximized: AtomicBool,
    pub sync: Mutex<SyncState>,
    pub update: Mutex<UpdateState>,
    pub update_asset: Mutex<Option<UpdateAsset>>,
    /// One in-flight sync per phone, de-duplicated like the JS `inflight` map.
    inflight: Mutex<HashMap<String, Arc<Mutex<Option<Result<SyncOutcome, String>>>>>>,
    pub export_seq: AtomicUsize,
}

impl AppState {
    pub fn new(data_dir: PathBuf, store: Store) -> Self {
        AppState {
            data_dir,
            store: Mutex::new(store),
            quitting: AtomicBool::new(false),
            maximized: AtomicBool::new(false),
            sync: Mutex::new(SyncState::default()),
            update: Mutex::new(UpdateState::default()),
            update_asset: Mutex::new(None),
            inflight: Mutex::new(HashMap::new()),
            export_seq: AtomicUsize::new(0),
        }
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }
}

// ----------------------------------------------------------------------- run

pub fn run() {
    // Profile recovery runs before the store is opened (it edits config.json).
    let app_data = migrate::app_data_root();
    let migration = migrate::recover_profile(&app_data, APP_NAME);
    if migration.phones > 0 || migration.messages > 0 || !migration.skipped.is_empty() {
        println!(
            "[migrate] phones={} messages={} from={:?} skipped={:?}",
            migration.phones, migration.messages, migration.from, migration.skipped
        );
    }

    let data_dir = resolve_data_dir(&app_data);
    if let Err(err) = std::fs::create_dir_all(&data_dir) {
        eprintln!("[boot] cannot create {}: {err}", data_dir.display());
    }
    let mut store = Store::new(&data_dir);
    store.prune();
    let lang = i18n::resolve_lang(&store.setting_str("lang", "auto"), &i18n::system_locale());
    i18n::set_lang(lang);

    let state = AppState::new(data_dir, store);

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            show_window(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            crate::commands::state_get,
            crate::commands::update_check,
            crate::commands::update_apply,
            crate::commands::window_minimize,
            crate::commands::window_toggle_maximize,
            crate::commands::window_close,
            crate::commands::window_is_maximized,
            crate::commands::phone_probe,
            crate::commands::phone_pair,
            crate::commands::phone_import,
            crate::commands::phone_remove,
            crate::commands::phone_rename,
            crate::commands::phone_health,
            crate::commands::sync_one,
            crate::commands::sync_all,
            crate::commands::settings_set,
            crate::commands::messages_seen,
            crate::commands::messages_clear,
            crate::commands::messages_export,
            crate::commands::app_open_data_dir,
            crate::commands::app_open_external,
            crate::commands::app_reveal_config,
        ])
        .setup(|app| {
            boot(app.handle());
            Ok(())
        });

    let app = builder
        .build(tauri::generate_context!())
        .expect("error while building the Tauri application");

    app.run(|app_handle, event| {
        if let RunEvent::Exit = event {
            let state = app_handle.state::<AppState>();
            state.quitting.store(true, Ordering::SeqCst);
            state.store.lock().unwrap().flush_db();
        }
    });
}

/// `%APPDATA%\Windows SMS Bridge`, or `SMSGLASS_DATA_DIR` when set (the old
/// client had the same override for development and tests).
pub fn resolve_data_dir(app_data: &Path) -> PathBuf {
    if let Ok(dir) = std::env::var("SMSGLASS_DATA_DIR") {
        let dir = dir.trim();
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    app_data.join(APP_NAME)
}

fn boot(app: &AppHandle) {
    let start_hidden = std::env::args().any(|arg| arg == "--hidden");
    create_main_window(app, start_hidden);
    crate::tray::refresh(app);
    start_scheduler(app);

    // Health probe of the first phone shortly after launch (main.js: 2.5 s).
    let first = {
        let state = app.state::<AppState>();
        let store = state.store.lock().unwrap();
        store
            .list_phones()
            .first()
            .and_then(|p| p.get("id").and_then(|v| v.as_str()).map(str::to_string))
    };
    if let Some(id) = first {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(2500)).await;
            let _ = tauri::async_runtime::spawn_blocking(move || {
                refresh_health(&handle, &id);
            })
            .await;
        });
    }

    // Silent update check 8 s after launch, startup sync 4 s after launch.
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(8000)).await;
        let handle = handle.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || {
            run_update_check(&handle);
        })
        .await;
    });

    let sync_on_start = {
        let state = app.state::<AppState>();
        let store = state.store.lock().unwrap();
        store.setting_bool("syncOnStart", true) && !store.list_phones().is_empty()
    };
    if sync_on_start {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(4000)).await;
            let running = handle.state::<AppState>().sync.lock().unwrap().running;
            if !running {
                let _ = tauri::async_runtime::spawn_blocking(move || {
                    let _ = sync_all(&handle, false, false);
                })
                .await;
            }
        });
    }
}

pub fn initial_window_size(app: &AppHandle) -> (f64, f64) {
    let area = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|monitor| {
            let scale = monitor.scale_factor();
            let size = monitor.size();
            (size.width as f64 / scale, size.height as f64 / scale)
        })
        .unwrap_or((1920.0, 1080.0));
    let width = std::env::var("SMSGLASS_WIN_W")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or_else(|| 1380.0_f64.min((area.0 - 40.0).max(1000.0)));
    let height = std::env::var("SMSGLASS_WIN_H")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or_else(|| 880.0_f64.min((area.1 - 30.0).max(660.0)));
    (width, height)
}

pub fn create_main_window(app: &AppHandle, start_hidden: bool) -> Option<WebviewWindow> {
    if let Some(existing) = app.get_webview_window("main") {
        return Some(existing);
    }
    let (width, height) = initial_window_size(app);
    let state = app.state::<AppState>();
    let hide_on_start = start_hidden || state.store.lock().unwrap().setting_bool("startHidden", false);

    let window = WebviewWindowBuilder::new(
        app,
        "main",
        WebviewUrl::App(PathBuf::from("ui/index.html")),
    )
    .title(APP_NAME)
    .inner_size(width, height)
    .min_inner_size(1000.0, 640.0)
    .decorations(false)
    .background_color(Color(7, 7, 11, 255))
    .visible(false)
    .on_page_load(|window, payload| {
        if payload.event() == PageLoadEvent::Finished {
            // Electron applied the stored zoom factor on did-finish-load.
            let handle = window.app_handle().clone();
            apply_ui_scale(&handle, stored_ui_scale(&handle));
        }
    })
    .build();

    let window = match window {
        Ok(window) => window,
        Err(err) => {
            eprintln!("[boot] window creation failed: {err}");
            return None;
        }
    };

    let handle = app.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::CloseRequested { api, .. } => {
            let state = handle.state::<AppState>();
            let quitting = state.quitting.load(Ordering::SeqCst);
            let close_to_tray = state.store.lock().unwrap().setting_bool("closeToTray", true);
            if !quitting && close_to_tray {
                api.prevent_close();
                if let Some(window) = handle.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            // With closeToTray off the Electron app closed the window but kept
            // running in the tray; the same happens here (no exit on drop).
        }
        WindowEvent::Resized(_) => {
            let state = handle.state::<AppState>();
            let maximized = handle
                .get_webview_window("main")
                .and_then(|window| window.is_maximized().ok())
                .unwrap_or(false);
            if state.maximized.swap(maximized, Ordering::SeqCst) != maximized {
                emit_event(&handle, "window:maximized", json!(maximized));
            }
        }
        _ => {}
    });

    if !hide_on_start {
        let _ = window.show();
        let _ = window.set_focus();
    }
    Some(window)
}

fn stored_ui_scale(app: &AppHandle) -> f64 {
    let state = app.state::<AppState>();
    let value = state.store.lock().unwrap().setting_num("uiScale", 1.1);
    value
}

// ------------------------------------------------------------------ snapshot

pub fn default_client_name() -> String {
    std::env::var("COMPUTERNAME")
        .ok()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| APP_NAME.to_string())
}

pub fn sync_snapshot(state: &AppState) -> Value {
    let sync = state.sync.lock().unwrap();
    let mut out = Map::new();
    out.insert("running".into(), json!(sync.running));
    out.insert("lastAt".into(), json!(sync.last_at));
    out.insert(
        "lastSummary".into(),
        sync.last_summary.clone().unwrap_or(Value::Null),
    );
    out.insert("errors".into(), json!(sync.errors));
    Value::Object(out)
}

pub fn update_snapshot(state: &AppState) -> Value {
    let update = state.update.lock().unwrap();
    let mut out = Map::new();
    out.insert("available".into(), json!(update.available));
    out.insert("version".into(), json!(update.version));
    out.insert("notes".into(), json!(update.notes));
    out.insert("checking".into(), json!(update.checking));
    out.insert(
        "progress".into(),
        match update.progress {
            Some((done, total)) => json!({ "done": done, "total": total }),
            None => Value::Null,
        },
    );
    out.insert("error".into(), json!(update.error));
    Value::Object(out)
}

pub fn snapshot(state: &AppState) -> Value {
    let (phones, settings, messages, stats) = {
        let store = state.store.lock().unwrap();
        let mut messages = store.all_messages();
        messages.truncate(MAX_SNAPSHOT_MESSAGES);
        let (total, phones_with_messages) = store.stats();
        (
            store.list_phones(),
            store.settings(),
            messages,
            json!({ "total": total, "phones": phones_with_messages }),
        )
    };
    let autostart = settings
        .get("autoStart")
        .map(|v| match v {
            Value::Bool(b) => *b,
            Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
            Value::String(s) => !s.is_empty() && s != "false",
            _ => false,
        })
        .unwrap_or(false);

    let mut app = Map::new();
    app.insert("version".into(), json!(APP_VERSION));
    app.insert("dataDir".into(), json!(state.data_dir.to_string_lossy()));
    app.insert("autostart".into(), json!(autostart));
    app.insert("clientName".into(), json!(default_client_name()));
    app.insert("update".into(), update_snapshot(state));

    let mut root = Map::new();
    root.insert("phones".into(), json!(phones));
    root.insert("messages".into(), json!(messages));
    root.insert("settings".into(), Value::Object(settings));
    root.insert("stats".into(), stats);
    root.insert("sync".into(), sync_snapshot(state));
    root.insert("app".into(), Value::Object(app));
    Value::Object(root)
}

// -------------------------------------------------------------------- events

pub fn emit_event(app: &AppHandle, channel: &str, payload: Value) {
    if let Err(err) = app.emit(channel, payload) {
        eprintln!("[event] {channel}: {err}");
    }
}

pub fn emit_state_changed(app: &AppHandle, reason: &str) {
    emit_event(app, "state:changed", json!({ "reason": reason }));
}

// -------------------------------------------------------------------- window

pub fn show_window(app: &AppHandle) {
    if app.get_webview_window("main").is_none() {
        create_main_window(app, false);
        return;
    }
    if let Some(window) = app.get_webview_window("main") {
        if window.is_minimized().unwrap_or(false) {
            let _ = window.unminimize();
        }
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// A phone app (or a second launch) asked for the window while it was hidden.
pub fn hide_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

pub fn minimize(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.minimize();
    }
}

pub fn toggle_maximize(app: &AppHandle) -> bool {
    let Some(window) = app.get_webview_window("main") else {
        return false;
    };
    if window.is_maximized().unwrap_or(false) {
        let _ = window.unmaximize();
    } else {
        let _ = window.maximize();
    }
    window.is_maximized().unwrap_or(false)
}

pub fn is_maximized(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .and_then(|window| window.is_maximized().ok())
        .unwrap_or(false)
}

/// The UI's own ✕ button: hide to tray, or really quit when that is disabled.
pub fn close_from_ui(app: &AppHandle) {
    let close_to_tray = {
        let state = app.state::<AppState>();
        let value = state.store.lock().unwrap().setting_bool("closeToTray", true);
        value
    };
    if close_to_tray {
        hide_window(app);
    } else {
        quit(app);
    }
}

pub fn quit(app: &AppHandle) {
    let state = app.state::<AppState>();
    state.quitting.store(true, Ordering::SeqCst);
    state.store.lock().unwrap().flush_db();
    app.exit(0);
}

/// Electron's `win.webContents.setZoomFactor(scale)`.
pub fn apply_ui_scale(app: &AppHandle, value: f64) -> f64 {
    let scale = value.clamp(0.85, 1.5);
    if let Some(window) = app.get_webview_window("main") {
        if let Err(err) = window.set_zoom(scale) {
            eprintln!("[zoom] {err}");
        }
    }
    scale
}

/// Interface language lives in settings but the native side needs it too
/// (tray menu, notifications, dialogs).
pub fn apply_language(app: &AppHandle) -> &'static str {
    let setting = {
        let state = app.state::<AppState>();
        let value = state.store.lock().unwrap().setting_str("lang", "auto");
        value
    };
    let lang = i18n::resolve_lang(&setting, &i18n::system_locale());
    i18n::set_lang(lang);
    crate::tray::refresh(app);
    lang
}

pub fn refresh_tray(app: &AppHandle) {
    crate::tray::refresh(app);
}

// --------------------------------------------------------------- health probe

pub fn refresh_health(app: &AppHandle, phone_id: &str) -> Option<Value> {
    let state = app.state::<AppState>();
    let (url, cert_pem) = {
        let store = state.store.lock().unwrap();
        let phone = store.get_phone(phone_id)?;
        (
            phone.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            phone
                .get("certPem")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        )
    };

    match crate::smsbridge::health(&url, cert_pem.as_deref(), false, 6000) {
        Ok(health) => {
            let mut stored = match health {
                Value::Object(map) => map,
                other => {
                    let mut map = Map::new();
                    map.insert("value".into(), other);
                    map
                }
            };
            stored.insert("fetchedAt".into(), json!(now_ms()));
            let stored = Value::Object(stored);
            let mut patch = Map::new();
            patch.insert("health".into(), stored);
            patch.insert("lastError".into(), json!(""));
            let mut store = state.store.lock().unwrap();
            store.update_phone(phone_id, &patch);
            Some(patch["health"].clone())
        }
        Err(err) => {
            let mut patch = Map::new();
            patch.insert("health".into(), Value::Null);
            patch.insert("lastError".into(), json!(err.message));
            let mut store = state.store.lock().unwrap();
            store.update_phone(phone_id, &patch);
            None
        }
    }
}

// ---------------------------------------------------------------------- sync

fn run_sync_task(
    app: &AppHandle,
    phone_id: &str,
    opts: SyncOptions,
) -> Result<SyncOutcome, String> {
    let state = app.state::<AppState>();
    let reused = {
        let mut inflight = state.inflight.lock().unwrap();
        match inflight.get(phone_id).cloned() {
            Some(cell) => Some(cell),
            None => {
                let cell: Arc<Mutex<Option<Result<SyncOutcome, String>>>> =
                    Arc::new(Mutex::new(None));
                inflight.insert(phone_id.to_string(), cell.clone());
                None
            }
        }
    };

    if let Some(cell) = reused {
        // Another sync for this phone is running: hand back its result, exactly
        // like the JS `inflight` promise map.
        let guard = cell.lock().unwrap();
        return match guard.as_ref() {
            Some(Ok(outcome)) => Ok(*outcome),
            Some(Err(err)) => Err(err.clone()),
            None => Err("sync already in progress".to_string()),
        };
    }

    let progress_app = app.clone();
    let outcome = {
        let mut store = state.store.lock().unwrap();
        sync_phone(&mut store, phone_id, opts, |progress| match progress {
            SyncProgress::Fetching {
                phone_id,
                mode,
                count,
            } => emit_event(
                &progress_app,
                "sync:progress",
                json!({ "phoneId": phone_id, "phase": "fetching", "mode": mode, "count": count }),
            ),
            SyncProgress::Done {
                phone_id,
                added,
                updated,
                total,
            } => emit_event(
                &progress_app,
                "sync:progress",
                json!({ "phoneId": phone_id, "phase": "done", "added": added, "updated": updated, "total": total }),
            ),
        })
        .map_err(|err| err.message.clone())
    };

    if let Some(cell) = state.inflight.lock().unwrap().remove(phone_id) {
        *cell.lock().unwrap() = Some(outcome.clone());
        // The cell may still be shared with a waiter that locked it before the
        // removal; keep the value alive for them by leaving the Arc intact.
        let _ = &cell;
    }
    outcome
}

/// Sync one phone. `full` = the deep/manual re-read; `throw_on_error` mirrors
/// the JS `manual` flag that decides whether the IPC call rejects.
pub fn sync_one(
    app: &AppHandle,
    phone_id: &str,
    full: bool,
    throw_on_error: bool,
) -> Result<Option<SyncOutcome>, BridgeError> {
    let state = app.state::<AppState>();
    let exists = {
        let store = state.store.lock().unwrap();
        store.get_phone(phone_id).is_some()
    };
    if !exists {
        return Ok(None);
    }

    let outcome = run_sync_task(
        app,
        phone_id,
        SyncOptions {
            full,
            days: None,
            since_id: None,
        },
    );

    match outcome {
        Ok(result) => {
            refresh_health(app, phone_id);
            after_sync(app, &[(phone_id.to_string(), result.added)]);
            Ok(Some(result))
        }
        Err(message) => {
            let unauthorized = message.to_lowercase().contains("unknown client")
                || message.to_lowercase().contains("bad signature")
                || message.to_lowercase().contains("revoked")
                || message.contains("401");
            let display = if unauthorized {
                t(
                    "配对凭据已失效：右键这台设备选「重新配对（换新密钥/证书）」",
                    &[],
                )
            } else {
                message.clone()
            };
            {
                let mut store = state.store.lock().unwrap();
                let mut patch = Map::new();
                patch.insert("lastError".into(), json!(display));
                patch.insert("authError".into(), json!(unauthorized));
                store.update_phone(phone_id, &patch);
            }
            {
                let mut sync = state.sync.lock().unwrap();
                sync.errors.push(json!({
                    "phoneId": phone_id,
                    "message": message,
                    "code": Value::Null
                }));
            }
            emit_event(
                app,
                "sync:error",
                json!({ "phoneId": phone_id, "message": message, "code": Value::Null }),
            );
            crate::tray::refresh(app);
            if throw_on_error {
                Err(BridgeError::new(message))
            } else {
                Ok(None)
            }
        }
    }
}

pub fn sync_all(app: &AppHandle, manual: bool, full: bool) -> Result<Option<Value>, String> {
    let state = app.state::<AppState>();
    {
        let sync = state.sync.lock().unwrap();
        if sync.running {
            return Ok(sync.last_summary.clone());
        }
    }
    let phones = {
        let store = state.store.lock().unwrap();
        store.list_phones()
    };
    if phones.is_empty() {
        if manual {
            return Err(t("还没有配对任何手机", &[]));
        }
        return Ok(None);
    }

    {
        let mut sync = state.sync.lock().unwrap();
        sync.running = true;
        sync.errors.clear();
    }
    emit_event(app, "sync:started", json!({ "count": phones.len() }));
    crate::tray::refresh(app);

    let mut results: Vec<(String, u64)> = Vec::new();
    for phone in &phones {
        let id = phone
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let name = phone
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        match run_sync_task(
            app,
            &id,
            SyncOptions {
                full,
                days: None,
                since_id: None,
            },
        ) {
            Ok(outcome) => {
                results.push((id.clone(), outcome.added));
                refresh_health(app, &id);
            }
            Err(message) => {
                let mut store = state.store.lock().unwrap();
                let mut patch = Map::new();
                patch.insert("lastError".into(), json!(message));
                store.update_phone(&id, &patch);
                drop(store);
                {
                    let mut sync = state.sync.lock().unwrap();
                    sync.errors.push(json!({ "phoneId": id, "name": name, "message": message }));
                }
                emit_event(
                    app,
                    "sync:error",
                    json!({ "phoneId": id, "name": name, "message": message, "code": Value::Null }),
                );
            }
        }
    }

    {
        let mut sync = state.sync.lock().unwrap();
        sync.running = false;
    }
    after_sync(app, &results);

    let (errors, summary) = {
        let sync = state.sync.lock().unwrap();
        (sync.errors.clone(), sync.last_summary.clone())
    };
    if manual && results.is_empty() && !errors.is_empty() {
        let first = errors
            .first()
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("sync failed")
            .to_string();
        return Err(first);
    }
    Ok(summary)
}

fn after_sync(app: &AppHandle, results: &[(String, u64)]) {
    let state = app.state::<AppState>();
    let added: u64 = results.iter().map(|(_, n)| *n).sum();
    let summary = {
        let mut sync = state.sync.lock().unwrap();
        sync.last_at = now_ms();
        let errors = sync.errors.clone();
        let summary = json!({
            "at": sync.last_at,
            "added": added,
            "phones": results.len(),
            "errors": errors
        });
        sync.last_summary = Some(summary.clone());
        summary
    };
    {
        let mut store = state.store.lock().unwrap();
        store.prune();
    }
    crate::notifications::notify_new_messages(app, added, results);
    crate::tray::refresh(app);
    emit_event(
        app,
        "state:changed",
        json!({ "reason": "sync", "summary": summary }),
    );
}

pub fn sync_running(app: &AppHandle) -> bool {
    app.state::<AppState>().sync.lock().unwrap().running
}

pub fn phone_count(app: &AppHandle) -> usize {
    let state = app.state::<AppState>();
    let store = state.store.lock().unwrap();
    store.list_phones().len()
}

pub fn last_sync_at(app: &AppHandle) -> i64 {
    app.state::<AppState>().sync.lock().unwrap().last_at
}

// -------------------------------------------------------------- tray actions

pub fn sync_all_from_tray(app: &AppHandle) {
    let handle = app.clone();
    std::thread::spawn(move || {
        let _ = sync_all(&handle, true, false);
    });
}

pub fn sync_device_from_tray(app: &AppHandle, phone_id: String) {
    let handle = app.clone();
    std::thread::spawn(move || {
        // main.js: showWindow() then syncOne(p.id, true) — a full sync.
        let _ = sync_one(&handle, &phone_id, true, false);
    });
}

pub fn set_autostart_from_tray(app: &AppHandle, enabled: bool) {
    let _ = apply_autostart(app, enabled);
    let settings = {
        let state = app.state::<AppState>();
        let value = state.store.lock().unwrap().settings();
        value
    };
    emit_event(app, "settings:update", json!(settings));
    crate::tray::refresh(app);
}

pub fn apply_autostart(app: &AppHandle, enabled: bool) -> Result<Map<String, Value>, String> {
    let state = app.state::<AppState>();
    let settings = {
        let mut store = state.store.lock().unwrap();
        let mut patch = Map::new();
        patch.insert("autoStart".into(), json!(enabled));
        store.update_settings(&patch)
    };
    if let Err(err) = crate::autostart::apply(enabled) {
        // Best effort, like the old `reg` helper: never fail the settings save.
        eprintln!("[autostart] {err}");
    }
    crate::tray::refresh(app);
    Ok(settings)
}

// ------------------------------------------------------------------ settings

pub fn apply_settings_patch(
    app: &AppHandle,
    patch: &Map<String, Value>,
) -> Result<Map<String, Value>, String> {
    let state = app.state::<AppState>();
    if let Some(auto_start) = patch.get("autoStart") {
        let enabled = match auto_start {
            Value::Bool(b) => *b,
            Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
            Value::String(s) => s == "true" || s == "1",
            _ => false,
        };
        let _ = apply_autostart(app, enabled);
    }

    let next = {
        let mut store = state.store.lock().unwrap();
        store.update_settings(patch)
    };

    if patch.contains_key("syncIntervalMin") {
        bump_scheduler();
    }
    if let Some(scale) = next.get("uiScale") {
        apply_ui_scale(app, scale.as_f64().unwrap_or(1.1));
    }
    if patch.contains_key("lang") {
        apply_language(app);
    }

    emit_event(app, "settings:update", json!(next));
    Ok(next)
}

// ----------------------------------------------------------------- scheduler

static SCHEDULER_GEN: AtomicU64 = AtomicU64::new(1);

pub fn bump_scheduler() {
    SCHEDULER_GEN.fetch_add(1, Ordering::SeqCst);
}

/// Replaces `setInterval(...)` from main.js: a 1 s tick that fires a background
/// sync once the configured interval has elapsed. Changing the interval resets
/// the countdown (generation bump), like clearing and recreating the timer.
pub fn start_scheduler(app: &AppHandle) {
    let handle = app.clone();
    std::thread::spawn(move || {
        let mut generation = SCHEDULER_GEN.load(Ordering::SeqCst);
        let mut last_fire = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(1000));
            let current = SCHEDULER_GEN.load(Ordering::SeqCst);
            if current != generation {
                generation = current;
                last_fire = Instant::now();
                continue;
            }
            let state = handle.state::<AppState>();
            let minutes = state.store.lock().unwrap().setting_num("syncIntervalMin", 0.0);
            if minutes <= 0.0 {
                continue;
            }
            let interval = Duration::from_millis((minutes.max(1.0) * 60_000.0) as u64);
            if last_fire.elapsed() < interval {
                continue;
            }
            last_fire = Instant::now();
            if state.sync.lock().unwrap().running {
                continue;
            }
            let handle = handle.clone();
            let _ = tauri::async_runtime::spawn_blocking(move || {
                let _ = sync_all(&handle, false, false);
            });
        }
    });
}

// ------------------------------------------------------------------- updater

/// Silent on startup, explicit when the user clicks "check for updates".
pub fn run_update_check(app: &AppHandle) -> Value {
    let state = app.state::<AppState>();
    {
        let mut update = state.update.lock().unwrap();
        if update.checking {
            drop(update);
            return update_snapshot(state.inner());
        }
        update.checking = true;
        update.error.clear();
    }
    emit_state_changed(app, "update");

    let result = crate::updater::check(APP_VERSION);
    let found = match result {
        Ok(found) => found,
        Err(err) => {
            let mut update = state.update.lock().unwrap();
            *update = UpdateState {
                checking: false,
                error: err,
                ..Default::default()
            };
            *state.update_asset.lock().unwrap() = None;
            drop(update);
            emit_state_changed(app, "update");
            return update_snapshot(state.inner());
        }
    };

    match found {
        Some(found) => {
            let mut update = state.update.lock().unwrap();
            *update = UpdateState {
                available: true,
                version: found.version.clone(),
                notes: found.notes.clone(),
                ..Default::default()
            };
            drop(update);
            *state.update_asset.lock().unwrap() = Some(found.asset);
        }
        None => {
            let mut update = state.update.lock().unwrap();
            *update = UpdateState::default();
            drop(update);
            *state.update_asset.lock().unwrap() = None;
        }
    }
    emit_state_changed(app, "update");
    update_snapshot(state.inner())
}

pub fn update_available(app: &AppHandle) -> Value {
    update_snapshot(app.state::<AppState>().inner())
}

pub fn apply_update(app: &AppHandle) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let asset = state
        .update_asset
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| t("没有可用的更新", &[]))?;

    let dir = state.data_dir.join("updates");
    {
        let mut update = state.update.lock().unwrap();
        update.progress = Some((0, asset.size));
    }
    emit_state_changed(app, "update");

    let progress_app = app.clone();
    let downloaded = crate::updater::download(&asset, &dir, move |done, total| {
        let progress_state = progress_app.state::<AppState>();
        {
            let mut update = progress_state.update.lock().unwrap();
            update.progress = Some((done, total));
        }
        emit_event(&progress_app, "update:progress", json!({ "done": done, "total": total }));
    });

    let file = match downloaded {
        Ok(file) => file,
        Err(err) => {
            let mut update = state.update.lock().unwrap();
            update.progress = None;
            update.error = err.clone();
            drop(update);
            emit_state_changed(app, "update");
            return Err(err);
        }
    };

    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from(APP_NAME));
    crate::updater::install_and_restart(&file, &exe);

    state.quitting.store(true, Ordering::SeqCst);
    let handle = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        handle.exit(0);
    });
    Ok(json!({ "ok": true }))
}

// ---------------------------------------------------------------- pair/import

/// TOFU probe: fetch the phone certificate and report its fingerprint so the
/// user can compare it with the one shown on the phone's pairing screen.
pub fn probe(url: &str) -> Result<Value, String> {
    let pin = crate::smsbridge::pin_certificate(url, 8000).map_err(|e| e.message)?;
    let health = crate::smsbridge::health(&pin.base, Some(&pin.pem), false, 6000)
        .map_err(|e| e.message)?;
    let mut out = match health {
        Value::Object(map) => map,
        other => {
            let mut map = Map::new();
            map.insert("value".into(), other);
            map
        }
    };
    out.insert("base".into(), json!(pin.base));
    out.insert("fingerprint".into(), json!(pin.pretty));
    out.insert("fingerprintHex".into(), json!(pin.fingerprint));
    Ok(Value::Object(out))
}

pub fn pair(
    app: &AppHandle,
    name: Option<String>,
    url: String,
    code: String,
    phone_id: Option<String>,
) -> Result<Value, String> {
    let pin = crate::smsbridge::pin_certificate(&url, 10000).map_err(|e| e.message)?;
    // fail fast when the phone is up but not healthy
    let info = crate::smsbridge::health(&pin.base, Some(&pin.pem), false, 6000)
        .map_err(|e| e.message)?;
    let creds = crate::smsbridge::pair(&pin.base, &code, &default_client_name(), Some(&pin.pem))
        .map_err(|e| e.message)?;

    let state = app.state::<AppState>();
    let mut id = phone_id.filter(|id| !id.is_empty());
    {
        let mut store = state.store.lock().unwrap();
        let re_pair = match id.as_deref() {
            Some(existing) => store.get_phone(existing).is_some(),
            None => false,
        };
        if re_pair {
            let existing = id.clone().unwrap();
            let mut health = match info.clone() {
                Value::Object(map) => map,
                other => {
                    let mut map = Map::new();
                    map.insert("value".into(), other);
                    map
                }
            };
            health.insert("fetchedAt".into(), json!(now_ms()));
            let mut patch = Map::new();
            patch.insert("url".into(), json!(creds.base_url));
            patch.insert("clientId".into(), json!(creds.client_id));
            patch.insert("clientName".into(), json!(creds.client_name));
            patch.insert("certPem".into(), json!(pin.pem));
            patch.insert("certFp".into(), json!(pin.fingerprint));
            patch.insert("health".into(), Value::Object(health));
            patch.insert("lastError".into(), json!(""));
            patch.insert("authError".into(), json!(false));
            for (k, v) in store.seal_token(&creds.token) {
                patch.insert(k, v);
            }
            store.update_phone(&existing, &patch);
        } else {
            let model = info
                .get("model")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let label = name
                .clone()
                .filter(|n| !n.is_empty())
                .or(model)
                .unwrap_or_else(|| "我的手机".to_string());
            let phone = store.add_phone(
                &label,
                &creds.base_url,
                &creds.client_id,
                &creds.token,
                &creds.client_name,
                &pin.pem,
                &pin.fingerprint,
            );
            id = phone
                .get("id")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            if let Some(new_id) = id.clone() {
                let mut health = match info.clone() {
                    Value::Object(map) => map,
                    other => {
                        let mut map = Map::new();
                        map.insert("value".into(), other);
                        map
                    }
                };
                health.insert("fetchedAt".into(), json!(now_ms()));
                let mut patch = Map::new();
                patch.insert("health".into(), Value::Object(health));
                store.update_phone(&new_id, &patch);
            }
        }
    }
    crate::tray::refresh(app);

    if let Some(id) = id.clone() {
        // first backfill; a failure here must not lose the pairing
        let _ = run_sync_task(
            app,
            &id,
            SyncOptions {
                full: true,
                days: None,
                since_id: None,
            },
        );
        refresh_health(app, &id);
    }
    emit_state_changed(app, "paired");

    let mut phone = {
        let state = app.state::<AppState>();
        let store = state.store.lock().unwrap();
        id.as_deref()
            .and_then(|id| store.get_phone_public(id))
            .unwrap_or(Value::Null)
    };
    if let Some(map) = phone.as_object_mut() {
        map.insert("fingerprint".into(), json!(pin.pretty));
        map.insert("fingerprintHex".into(), json!(pin.fingerprint));
    }
    Ok(phone)
}

pub fn import_phone(
    app: &AppHandle,
    name: Option<String>,
    url: String,
    client_id: String,
    token: String,
) -> Result<Value, String> {
    let pin = crate::smsbridge::pin_certificate(&url, 10000).map_err(|e| e.message)?;
    // validate by a signed call, over the pinned certificate only
    let who = crate::smsbridge::BridgeClient::new(&pin.base, &client_id, &token, Some(&pin.pem))
        .map_err(|e| e.message)?
        .whoami()
        .map_err(|e| e.message)?;
    let client_name = who
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let phone = {
        let state = app.state::<AppState>();
        let mut store = state.store.lock().unwrap();
        store.add_phone(
            name.as_deref().filter(|n| !n.is_empty()).unwrap_or("我的手机"),
            &pin.base,
            &client_id,
            &token,
            &client_name,
            &pin.pem,
            &pin.fingerprint,
        )
    };
    let id = phone
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_default();
    refresh_health(app, &id);
    crate::tray::refresh(app);
    let _ = run_sync_task(
        app,
        &id,
        SyncOptions {
            full: true,
            days: None,
            since_id: None,
        },
    );
    emit_state_changed(app, "imported");

    let mut phone = {
        let state = app.state::<AppState>();
        let store = state.store.lock().unwrap();
        store.get_phone_public(&id).unwrap_or(Value::Null)
    };
    if let Some(map) = phone.as_object_mut() {
        map.insert("fingerprint".into(), json!(pin.pretty));
    }
    Ok(phone)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_dir_defaults_to_appdata_name() {
        let root = Path::new("C:/Users/x/AppData/Roaming");
        std::env::remove_var("SMSGLASS_DATA_DIR");
        assert_eq!(
            resolve_data_dir(root),
            root.join("Windows SMS Bridge"),
            "must match Electron's userData path"
        );
    }

    #[test]
    fn ui_scale_is_clamped_like_main_js() {
        // main.js: Math.min(1.5, Math.max(0.85, Number(value) || 1))
        let clamp = |v: f64| v.clamp(0.85, 1.5);
        assert_eq!(clamp(0.5), 0.85);
        assert_eq!(clamp(2.0), 1.5);
        assert_eq!(clamp(1.1), 1.1);
    }
}

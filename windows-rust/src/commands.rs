//! The 22 IPC commands — one per channel of `windows/preload.js`, same names,
//! same request/response shapes. The channel names are the contract with the
//! renderer (`web/ui/tauri.js` calls these exact strings):
//!
//!   state:get  update:check  update:apply
//!   window:minimize  window:toggle-maximize  window:close  window:is-maximized
//!   phone:probe  phone:pair  phone:import  phone:remove  phone:rename  phone:health
//!   sync:one  sync:all  settings:set
//!   messages:seen  messages:clear  messages:export
//!   app:open-data-dir  app:open-external  app:reveal-config
//!
//! `tauri::command` is configured with the default camelCase argument mapping so
//! the JS payloads (`{ phoneId }`, `{ clientId }`, …) land in the snake_case
//! parameters below.

use serde_json::{Map, Value};
use tauri::{AppHandle, Manager};

use crate::app::{self, AppState};

async fn blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|err| format!("task failed: {err}"))
}

// ------------------------------------------------------------------ snapshot

#[tauri::command]
pub async fn state_get(app: AppHandle) -> Result<Value, String> {
    let app = app.clone();
    Ok(blocking(move || app::snapshot(app.state::<AppState>().inner())).await?)
}

// ------------------------------------------------------------------- updater

#[tauri::command]
pub async fn update_check(app: AppHandle) -> Result<Value, String> {
    Ok(blocking(move || app::run_update_check(&app)).await?)
}

#[tauri::command]
pub async fn update_apply(app: AppHandle) -> Result<Value, String> {
    blocking(move || app::apply_update(&app)).await?
}

// -------------------------------------------------------------------- window

#[tauri::command]
pub fn window_minimize(app: AppHandle) {
    app::minimize(&app);
}

#[tauri::command]
pub fn window_toggle_maximize(app: AppHandle) -> bool {
    app::toggle_maximize(&app)
}

#[tauri::command]
pub fn window_close(app: AppHandle) {
    app::close_from_ui(&app);
}

#[tauri::command]
pub fn window_is_maximized(app: AppHandle) -> bool {
    app::is_maximized(&app)
}

// -------------------------------------------------------------------- phones

#[tauri::command]
pub async fn phone_probe(url: String) -> Result<Value, String> {
    blocking(move || app::probe(&url)).await?
}

#[tauri::command]
pub async fn phone_pair(
    app: AppHandle,
    name: Option<String>,
    url: String,
    code: String,
    phone_id: Option<String>,
) -> Result<Value, String> {
    blocking(move || app::pair(&app, name, url, code, phone_id)).await?
}

#[tauri::command]
pub async fn phone_import(
    app: AppHandle,
    name: Option<String>,
    url: String,
    client_id: String,
    token: String,
) -> Result<Value, String> {
    blocking(move || app::import_phone(&app, name, url, client_id, token)).await?
}

#[tauri::command]
pub async fn phone_remove(app: AppHandle, id: String) -> Result<Value, String> {
    let phones = blocking(move || {
        let state = app.state::<AppState>();
        let phones = {
            let mut store = state.store.lock().unwrap();
            store.remove_phone(&id);
            store.list_phones()
        };
        crate::tray::refresh(&app);
        phones
    })
    .await?;
    Ok(Value::Array(phones))
}

#[tauri::command]
pub async fn phone_rename(app: AppHandle, id: String, name: String) -> Result<Value, String> {
    let updated = blocking(move || {
        let state = app.state::<AppState>();
        let mut patch = Map::new();
        patch.insert("name".into(), Value::String(name));
        let mut store = state.store.lock().unwrap();
        store.update_phone(&id, &patch)
    })
    .await?;
    Ok(updated.unwrap_or(Value::Null))
}

#[tauri::command]
pub async fn phone_health(app: AppHandle, id: String) -> Result<Value, String> {
    let handle = app.clone();
    let health = blocking(move || app::refresh_health(&handle, &id)).await?;
    app::emit_state_changed(&app, "health");
    Ok(health.unwrap_or(Value::Null))
}

// --------------------------------------------------------------------- sync

#[tauri::command]
pub async fn sync_one(app: AppHandle, id: String) -> Result<Value, String> {
    let phone_id = id.clone();
    let (outcome, state) = blocking(move || {
        let outcome = app::sync_one(&app, &phone_id, false, true).map_err(|err| err.message)?;
        Ok::<_, String>((outcome, app::snapshot(app.state::<AppState>().inner())))
    })
    .await??;
    let result = match outcome {
        Some(outcome) => serde_json::json!({
            "phoneId": id,
            "mode": Value::Null,
            "added": outcome.added,
            "updated": outcome.updated,
            "total": outcome.total,
            "fetched": outcome.fetched,
            "lastSmsId": outcome.last_sms_id
        }),
        None => Value::Null,
    };
    Ok(serde_json::json!({ "result": result, "state": state }))
}

#[tauri::command]
pub async fn sync_all(app: AppHandle) -> Result<Value, String> {
    let (summary, state) = blocking(move || {
        let summary = app::sync_all(&app, true, false)?;
        Ok::<_, String>((summary, app::snapshot(app.state::<AppState>().inner())))
    })
    .await??;
    Ok(serde_json::json!({ "state": state, "summary": summary }))
}

// ----------------------------------------------------------------- settings

#[tauri::command]
pub async fn settings_set(
    app: AppHandle,
    patch: Map<String, Value>,
) -> Result<Value, String> {
    let next = blocking(move || app::apply_settings_patch(&app, &patch)).await?;
    Ok(Value::Object(next?))
}

#[tauri::command]
pub async fn messages_seen(app: AppHandle, address: String) -> Result<u64, String> {
    let seen = blocking(move || {
        let state = app.state::<AppState>();
        let mut store = state.store.lock().unwrap();
        store.mark_seen(&address)
    })
    .await?;
    Ok(seen)
}

// ----------------------------------------------------------------- messages

#[tauri::command]
pub async fn messages_clear(app: AppHandle, phone_id: Option<String>) -> Result<Value, String> {
    let state = blocking(move || {
        let state = app.state::<AppState>();
        {
            let mut store = state.store.lock().unwrap();
            store.clear_messages(phone_id.as_deref());
        }
        app::snapshot(state.inner())
    })
    .await?;
    Ok(state)
}

#[tauri::command]
pub async fn messages_export(app: AppHandle, phone_id: Option<String>) -> Result<Value, String> {
    blocking(move || crate::export::export_messages(&app, phone_id.as_deref())).await?
}

// -------------------------------------------------------------- shell / misc

#[tauri::command]
pub async fn app_open_data_dir(app: AppHandle) -> Result<bool, String> {
    Ok(blocking(move || {
        let state = app.state::<AppState>();
        crate::shell_ops::open_path(&state.data_dir.clone())
    })
    .await?)
}

#[tauri::command]
pub async fn app_open_external(url: String) -> Result<bool, String> {
    Ok(blocking(move || {
        let lower = url.to_ascii_lowercase();
        if !(lower.starts_with("http://") || lower.starts_with("https://")) {
            return false;
        }
        crate::shell_ops::open_external(&url)
    })
    .await?)
}

#[tauri::command]
pub async fn app_reveal_config(app: AppHandle) -> Result<bool, String> {
    Ok(blocking(move || {
        let state = app.state::<AppState>();
        let path = state.data_dir.join("config.json");
        crate::shell_ops::reveal_in_folder(&path)
    })
    .await?)
}

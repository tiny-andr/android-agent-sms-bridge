//! New-SMS desktop notifications — ported from `notifyNewMessages` in
//! `windows/main.js`: only when there were new messages, the setting is on, and
//! the window is hidden. Up to three inbox messages per phone are considered,
//! the newest first, and the first one provides the body (120 chars).

use tauri::Manager;

use crate::app::AppState;
use crate::i18n::t;
use tauri_plugin_notification::NotificationExt;

pub fn notify_new_messages(app: &tauri::AppHandle, added: u64, results: &[(String, u64)]) {
    if added == 0 {
        return;
    }
    let state = app.state::<AppState>();
    let (notify_new, visible) = {
        let store = state.store.lock().unwrap();
        let notify = store.setting_bool("notifyNew", true);
        (notify, window_visible(app))
    };
    if !notify_new || visible {
        return;
    }

    let mut fresh = Vec::new();
    for (phone_id, phone_added) in results {
        if *phone_added == 0 {
            continue;
        }
        let store = state.store.lock().unwrap();
        let list: Vec<serde_json::Value> = store
            .messages_of(phone_id)
            .into_iter()
            .filter(|m| m.get("type").and_then(|v| v.as_str()) == Some("inbox"))
            .take(3)
            .collect();
        fresh.extend(list);
    }
    let Some(first) = fresh.first() else {
        return;
    };

    let address = first
        .get("address")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let title = if added == 1 {
        t("新短信 · {addr}", &[("addr", &address)])
    } else {
        t("{n} 条新短信", &[("n", &added.to_string())])
    };
    let body: String = first
        .get("body")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .chars()
        .take(120)
        .collect();

    if let Err(err) = app
        .notification()
        .builder()
        .title(title)
        .body(body)
        .show()
    {
        eprintln!("[notify] {err}");
    }
}

fn window_visible(app: &tauri::AppHandle) -> bool {
    app.get_webview_window("main")
        .map(|window| window.is_visible().unwrap_or(false))
        .unwrap_or(false)
}

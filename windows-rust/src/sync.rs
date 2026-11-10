//! Sync engine — pulls SMS from paired phones into the local cache.
//!
//! Ported 1:1 from the Electron build's `src/core/sync.js`.
//!
//! Two modes:
//!   full        re-read the last N days (used on first sync / manual deep sync)
//!   incremental only messages with _id > lastSmsId (cheap, the default)
//!
//! The phone clamps `days`/`limit` server-side and allows 60 req/min per client,
//! so we page with limit=200 and stay well under that.

use serde_json::{json, Map, Value};

use crate::smsbridge::{BridgeClient, BridgeError, SmsQuery};
use crate::store::{now_ms, Store};
use crate::i18n::t;

pub const PAGE_SIZE: i64 = 200;
pub const MAX_PAGES: usize = 20;

#[derive(Debug, Clone, Copy)]
pub struct SyncOutcome {
    pub added: u64,
    pub updated: u64,
    pub total: u64,
    pub fetched: u64,
    pub last_sms_id: i64,
}

#[derive(Debug, Clone, Default)]
pub struct SyncOptions {
    pub full: bool,
    pub days: Option<i64>,
    pub since_id: Option<i64>,
}

/// `normalize(row)` from sync.js — the shape cached in `messages.json`.
pub fn normalize(row: &Value) -> Value {
    let get = |k: &str| row.get(k).cloned().unwrap_or(Value::Null);
    let truthy_string = |v: Value| match v {
        Value::String(s) => s,
        _ => String::new(),
    };
    let truthy_number = |v: Value| match v {
        Value::Number(n) => Value::Number(n),
        _ => json!(0),
    };

    let mut out = Map::new();
    out.insert("id".into(), get("id"));
    out.insert("address".into(), json!(truthy_string(get("address"))));
    out.insert("body".into(), json!(truthy_string(get("body"))));
    out.insert("date".into(), truthy_number(get("date")));
    out.insert(
        "type".into(),
        match get("type") {
            Value::String(s) if !s.is_empty() => json!(s),
            _ => json!("inbox"),
        },
    );
    out.insert(
        "slot".into(),
        match get("slot") {
            Value::Number(n) => json!(n),
            _ => json!(-1),
        },
    );
    out.insert(
        "subId".into(),
        match row.get("sub_id") {
            Some(Value::Null) | None => json!(0),
            Some(v) => v.clone(),
        },
    );
    out.insert("carrier".into(), json!(truthy_string(get("carrier"))));
    out.insert("simNumber".into(), json!(truthy_string(get("sim_number"))));
    out.insert(
        "read".into(),
        json!(match get("read") {
            Value::Bool(b) => b,
            Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
            Value::String(s) => !s.is_empty(),
            _ => false,
        }),
    );
    Value::Object(out)
}

fn row_id(row: &Value) -> i64 {
    row.get("id")
        .and_then(|v| v.as_i64().or_else(|| v.as_u64().map(|u| u as i64)))
        .unwrap_or(0)
}

fn fetch_pages(
    client: &BridgeClient,
    base: SyncOptions,
    mut on_page: impl FnMut(usize, &Value),
) -> Result<Vec<Value>, BridgeError> {
    let mut rows: Vec<Value> = Vec::new();
    for page in 0..MAX_PAGES {
        let mut query = SmsQuery::new(PAGE_SIZE);
        query.box_ = "all".into();
        query.days = base.days;
        query.since_id = base.since_id;
        query.offset = (page as i64) * PAGE_SIZE;
        let res = client.sms(&query)?;
        let list: Vec<Value> = res
            .get("messages")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let short = (list.len() as i64) < PAGE_SIZE;
        rows.extend(list);
        on_page(rows.len(), &res);
        if short {
            break;
        }
    }
    Ok(rows)
}

#[derive(Debug, Clone)]
pub enum SyncProgress {
    Fetching {
        phone_id: String,
        mode: &'static str,
        count: usize,
    },
    Done {
        phone_id: String,
        added: u64,
        updated: u64,
        total: u64,
    },
}

/// The full one-phone sync. Progress callbacks mirror the `sync:progress`
/// event payload (`phase` = fetching|done) so the UI renders the same text.
pub fn sync_phone(
    store: &mut Store,
    phone_id: &str,
    opts: SyncOptions,
    mut on_progress: impl FnMut(SyncProgress),
) -> Result<SyncOutcome, BridgeError> {
    let phone = store
        .phone_raw(phone_id)
        .ok_or_else(|| BridgeError::new(t("手机不存在", &[])))?;
    let token = store.token_of(phone_id);
    if token.is_empty() {
        return Err(BridgeError::new(t("这台手机还没有配对凭据，请重新配对", &[])));
    }

    let url = phone.get("url").and_then(|v| v.as_str()).unwrap_or("");
    let client_id = phone
        .get("clientId")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let cert_pem = phone.get("certPem").and_then(|v| v.as_str());
    let client = BridgeClient::new(url, client_id, &token, cert_pem)?;

    let since_id = phone
        .get("lastSmsId")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    // Rows cached before 1.1.0 have no simNumber, and an incremental sync would
    // never backfill them. Do one full pass per phone and remember it.
    let sim_backfilled = phone
        .get("simBackfilled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let needs_backfill = !sim_backfilled && store.message_count(phone_id) > 0;
    let mode = if opts.full || since_id == 0 || needs_backfill {
        "full"
    } else {
        "incremental"
    };

    let days = if mode == "full" {
        let fallback = store.setting_num("pullDays", 30.0) as i64;
        let value = opts.days.unwrap_or(fallback);
        if value == 0 {
            30
        } else {
            value
        }
    } else {
        0
    };

    let base = SyncOptions {
        full: mode == "full",
        days: if mode == "full" { Some(days) } else { None },
        ..Default::default()
    };

    on_progress(SyncProgress::Fetching {
        phone_id: phone_id.to_string(),
        mode,
        count: 0,
    });
    let rows = fetch_pages(&client, base, |n, _res| {
        on_progress(SyncProgress::Fetching {
            phone_id: phone_id.to_string(),
            mode,
            count: n,
        });
    })?;
    let normalized: Vec<Value> = rows.iter().map(normalize).collect();

    let result = store.upsert_messages(phone_id, &normalized);
    let max_id = normalized
        .iter()
        .map(row_id)
        .chain(std::iter::once(since_id))
        .max()
        .unwrap_or(since_id);

    let mut patch = Map::new();
    patch.insert("lastSmsId".into(), json!(max_id));
    patch.insert("lastSyncAt".into(), json!(now_ms()));
    patch.insert("lastError".into(), json!(""));
    patch.insert("lastMode".into(), json!(mode));
    patch.insert("simBackfilled".into(), json!(true));
    let _ = store.update_phone(phone_id, &patch);

    on_progress(SyncProgress::Done {
        phone_id: phone_id.to_string(),
        added: result.added,
        updated: result.updated,
        total: result.total,
    });

    Ok(SyncOutcome {
        added: result.added,
        updated: result.updated,
        total: result.total,
        fetched: normalized.len() as u64,
        last_sms_id: max_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_matches_sync_js() {
        let raw = json!({
            "id": 7,
            "address": "10086",
            "body": "hi",
            "date": 1700000000000i64,
            "type": "inbox",
            "slot": 1,
            "sub_id": 2,
            "carrier": "中国电信",
            "sim_number": "+86138",
            "read": true,
            "date_iso": "2026-11-04 22:46:40",
            "extra": "dropped"
        });
        let row = normalize(&raw);
        let keys: Vec<&str> = row.as_object().unwrap().keys().map(|k| k.as_str()).collect();
        assert_eq!(
            keys,
            [
                "id", "address", "body", "date", "type", "slot", "subId", "carrier", "simNumber",
                "read"
            ],
            "field order and dropped fields must match sync.js"
        );
        assert_eq!(row["subId"], json!(2));
        assert_eq!(row["simNumber"], json!("+86138"));

        let sparse = normalize(&json!({ "id": 8 }));
        assert_eq!(sparse["slot"], json!(-1), "non-numeric slot → -1");
        assert_eq!(sparse["subId"], json!(0), "missing sub_id → 0");
        assert_eq!(sparse["type"], json!("inbox"));
        assert_eq!(sparse["read"], json!(false));
    }
}

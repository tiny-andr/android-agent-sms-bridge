//! Local persistence, byte-compatible with the Electron build.
//!
//! Two files under `%APPDATA%\Windows SMS Bridge\`:
//!   · `config.json`   `{version, settings, phones[]}` — pretty-printed (2 spaces)
//!   · `messages.json` `{version, messages: {phoneId: [row]}}` — compact
//!
//! An existing install must keep every cached SMS and every pairing, so the
//! writer reproduces the JavaScript serialisation exactly (field order included:
//! `serde_json` is compiled with `preserve_order` for that reason) and unknown
//! keys on phone records are preserved verbatim.
//!
//! The pairing token is never written in the clear: `tokenEnc` carries the
//! DPAPI-sealed blob, see `dpapi.rs`.

use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::{json, Map, Value};

use crate::dpapi::{write_atomic, TokenCrypto};

pub const CONFIG_VERSION: i64 = 1;

pub fn empty_db() -> Value {
    json!({ "version": 1, "messages": {} })
}

/// Field order matters: `config.json` is compared field-by-field by humans and
/// by the old client, so keep it identical to `DEFAULT_SETTINGS` in store.js.
pub fn default_settings() -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("autoStart".into(), json!(false)); // 开机自启
    m.insert("startHidden".into(), json!(false)); // 启动后最小化到托盘
    m.insert("syncIntervalMin".into(), json!(15)); // 0 = 关闭定时同步
    m.insert("syncOnStart".into(), json!(true)); // 启动后自动同步一次
    m.insert("notifyNew".into(), json!(true)); // 新短信桌面通知
    m.insert("pullDays".into(), json!(30)); // 每次从手机拉取的时间范围（天）
    m.insert("keepDays".into(), json!(0)); // 本地保留天数，0 = 永久
    m.insert("closeToTray".into(), json!(true)); // 关闭窗口时最小化到托盘
    m.insert("theme".into(), json!("dark")); // 'dark' | 'light'
    m.insert("uiScale".into(), json!(1.1)); // 0.85 – 1.5
    m.insert("lang".into(), json!("auto")); // 'auto' | 'zh' | 'en' | 'ja'
    m
}

pub struct Store {
    pub dir: PathBuf,
    pub crypto: TokenCrypto,
    config: Value,
    db: Value,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct UpsertResult {
    pub added: u64,
    pub updated: u64,
    pub total: u64,
}

impl Store {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let crypto = TokenCrypto::load(&dir);
        let mut store = Store {
            dir,
            crypto,
            config: Value::Null,
            db: Value::Null,
        };
        store.load();
        store
    }

    fn config_path(&self) -> PathBuf {
        self.dir.join("config.json")
    }

    fn db_path(&self) -> PathBuf {
        self.dir.join("messages.json")
    }

    // ------------------------------------------------------------------ load

    fn load(&mut self) {
        let config: Value = std::fs::read_to_string(self.config_path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_else(|| json!({ "version": CONFIG_VERSION, "settings": {}, "phones": [] }));

        let mut settings = default_settings();
        if let Some(saved) = config.get("settings").and_then(|v| v.as_object()) {
            for (k, v) in saved {
                settings.insert(k.clone(), v.clone());
            }
        }
        let phones = config
            .get("phones")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        // Keep unknown root keys (the migration markers written by
        // migrate::recover_profile must survive a save) and keep their order.
        let mut root = config.as_object().cloned().unwrap_or_default();
        root.insert("version".into(), json!(CONFIG_VERSION));
        root.insert("settings".into(), Value::Object(settings));
        root.insert("phones".into(), Value::Array(phones));
        self.config = Value::Object(root);

        let db: Value = std::fs::read_to_string(self.db_path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_else(empty_db);
        let db = match db {
            Value::Object(mut m) => {
                let needs_messages = !matches!(m.get("messages"), Some(Value::Object(_)));
                if needs_messages {
                    m.insert("messages".into(), json!({}));
                }
                Value::Object(m)
            }
            _ => empty_db(),
        };
        self.db = db;

        // migrate plaintext tokens to DPAPI, plain-http urls to https
        let mut migrated = false;
        let crypto = self.crypto.clone();
        for phone in self.phones_raw_mut() {
            let Some(obj) = phone.as_object_mut() else { continue };
            if let Some(url) = obj.get("url").and_then(|v| v.as_str()).map(str::to_string) {
                if url.to_lowercase().starts_with("http://") {
                    let fixed = format!("https://{}", &url[7..]);
                    obj.insert("url".into(), json!(fixed));
                    migrated = true;
                }
            }
            if !matches!(obj.get("certPem"), Some(Value::String(_))) {
                obj.insert("certPem".into(), json!(""));
            }
            if !matches!(obj.get("certFp"), Some(Value::String(_))) {
                obj.insert("certFp".into(), json!(""));
            }
            let raw = obj
                .get("tokenRaw")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            if let Some(raw) = raw {
                if crypto.available() {
                    let sealed = crypto.seal(&raw);
                    if sealed
                        .get("tokenEnc")
                        .and_then(|v| v.as_str())
                        .map(|s| !s.is_empty())
                        .unwrap_or(false)
                    {
                        for (k, v) in sealed {
                            obj.insert(k, v);
                        }
                        migrated = true;
                    }
                }
            }
        }
        if migrated {
            self.save_config();
        }
    }

    // ------------------------------------------------------------------ save

    pub fn save_config(&self) {
        let text = serde_json::to_string_pretty(&self.config).unwrap_or_default();
        if let Err(err) = write_atomic(&self.config_path(), &text) {
            eprintln!("[store] config save failed: {err}");
        }
    }

    pub fn flush_db(&self) {
        let text = serde_json::to_string(&self.db).unwrap_or_default();
        if let Err(err) = write_atomic(&self.db_path(), &text) {
            eprintln!("[store] db save failed: {err}");
        }
    }

    // -------------------------------------------------------------- settings

    pub fn settings(&self) -> Map<String, Value> {
        self.config
            .get("settings")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default()
    }

    pub fn update_settings(&mut self, patch: &Map<String, Value>) -> Map<String, Value> {
        if let Some(settings) = self
            .config
            .get_mut("settings")
            .and_then(|v| v.as_object_mut())
        {
            for (k, v) in patch {
                settings.insert(k.clone(), v.clone());
            }
        }
        self.save_config();
        self.settings()
    }

    pub fn setting_bool(&self, key: &str, fallback: bool) -> bool {
        self.config
            .get("settings")
            .and_then(|s| s.get(key))
            .map(|v| match v {
                Value::Bool(b) => *b,
                Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
                Value::String(s) => !s.is_empty() && s != "false" && s != "0",
                _ => fallback,
            })
            .unwrap_or(fallback)
    }

    pub fn setting_num(&self, key: &str, fallback: f64) -> f64 {
        self.config
            .get("settings")
            .and_then(|s| s.get(key))
            .and_then(|v| v.as_f64())
            .unwrap_or(fallback)
    }

    pub fn setting_str(&self, key: &str, fallback: &str) -> String {
        self.config
            .get("settings")
            .and_then(|s| s.get(key))
            .and_then(|v| v.as_str())
            .unwrap_or(fallback)
            .to_string()
    }

    // ---------------------------------------------------------------- phones

    fn phones_raw(&self) -> &Vec<Value> {
        static EMPTY: Vec<Value> = Vec::new();
        self.config
            .get("phones")
            .and_then(|v| v.as_array())
            .unwrap_or(&EMPTY)
    }

    fn phones_raw_mut(&mut self) -> std::slice::IterMut<'_, Value> {
        match self.config.get_mut("phones").and_then(|v| v.as_array_mut()) {
            Some(list) => list.iter_mut(),
            None => [].iter_mut(),
        }
    }

    pub fn list_phones(&self) -> Vec<Value> {
        self.phones_raw().iter().map(public_phone).collect()
    }

    pub fn get_phone(&self, id: &str) -> Option<&Value> {
        self.phones_raw()
            .iter()
            .find(|p| p.get("id").and_then(|v| v.as_str()) == Some(id))
    }

    pub fn get_phone_public(&self, id: &str) -> Option<Value> {
        self.get_phone(id).map(public_phone)
    }

    pub fn add_phone(
        &mut self,
        name: &str,
        url: &str,
        client_id: &str,
        token: &str,
        client_name: &str,
        cert_pem: &str,
        cert_fp: &str,
    ) -> Value {
        let mut phone = Map::new();
        phone.insert("id".into(), json!(uuid_v4()));
        phone.insert(
            "name".into(),
            json!(if name.is_empty() { "我的手机" } else { name }),
        );
        phone.insert("url".into(), json!(url));
        phone.insert("clientId".into(), json!(client_id));
        phone.insert("clientName".into(), json!(client_name));
        phone.insert("createdAt".into(), json!(now_ms()));
        phone.insert("lastSyncAt".into(), json!(0));
        phone.insert("lastSeq".into(), json!(0));
        phone.insert("lastSmsId".into(), json!(0));
        phone.insert("certPem".into(), json!(cert_pem));
        phone.insert("certFp".into(), json!(cert_fp));
        for (k, v) in self.crypto.seal(token) {
            phone.insert(k, v);
        }
        let record = Value::Object(phone);
        let public = public_phone(&record);
        if let Some(list) = self.config.get_mut("phones").and_then(|v| v.as_array_mut()) {
            list.push(record);
        }
        self.save_config();
        public
    }

    pub fn update_phone(&mut self, id: &str, patch: &Map<String, Value>) -> Option<Value> {
        let updated = {
            let phones = self.config.get_mut("phones")?.as_array_mut()?;
            let phone = phones
                .iter_mut()
                .find(|p| p.get("id").and_then(|v| v.as_str()) == Some(id));
            let phone = phone?;
            let obj = phone.as_object_mut()?;
            for (k, v) in patch {
                obj.insert(k.clone(), v.clone());
            }
            public_phone(phone)
        };
        self.save_config();
        Some(updated)
    }

    pub fn remove_phone(&mut self, id: &str) {
        if let Some(phones) = self.config.get_mut("phones").and_then(|v| v.as_array_mut()) {
            phones.retain(|p| p.get("id").and_then(|v| v.as_str()) != Some(id));
        }
        if let Some(messages) = self.db.get_mut("messages").and_then(|v| v.as_object_mut()) {
            messages.remove(id);
        }
        // Remember the deletion: profile recovery must never resurrect this
        // device (the Electron build did, on every restart).
        if let Some(root) = self.config.as_object_mut() {
            if !matches!(root.get("removedPhoneIds"), Some(Value::Array(_))) {
                root.insert("removedPhoneIds".into(), json!([]));
            }
            if let Some(list) = root
                .get_mut("removedPhoneIds")
                .and_then(|v| v.as_array_mut())
            {
                if !list.iter().any(|v| v.as_str() == Some(id)) {
                    list.push(json!(id));
                }
            }
        }
        self.save_config();
        self.flush_db();
    }

    /// Token as a plaintext string, exactly like the Electron `tokenOf`.
    pub fn token_of(&self, id: &str) -> String {
        let Some(phone) = self.get_phone(id) else {
            return String::new();
        };
        let enc = phone.get("tokenEnc").and_then(|v| v.as_str()).unwrap_or("");
        if !enc.is_empty() {
            let opened = self.crypto.open(enc);
            return opened;
        }
        phone
            .get("tokenRaw")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    }

    pub fn seal_token(&self, token: &str) -> Map<String, Value> {
        self.crypto.seal(token)
    }

    // -------------------------------------------------------------- messages

    pub fn upsert_messages(&mut self, phone_id: &str, rows: &[Value]) -> UpsertResult {
        let messages = self
            .db
            .get_mut("messages")
            .and_then(|v| v.as_object_mut())
            .expect("messages object");

        let bucket = messages
            .entry(phone_id.to_string())
            .or_insert_with(|| json!([]));
        if !bucket.is_array() {
            *bucket = json!([]);
        }
        let list = bucket.as_array_mut().expect("message bucket");

        let mut index: HashMap<i64, usize> = HashMap::with_capacity(list.len());
        for (i, m) in list.iter().enumerate() {
            if let Some(id) = message_id(m) {
                index.insert(id, i);
            }
        }

        let mut added = 0u64;
        let mut updated = 0u64;
        for row in rows {
            let Some(id) = message_id(row) else { continue };
            match index.get(&id) {
                None => {
                    index.insert(id, list.len());
                    list.push(row.clone());
                    added += 1;
                }
                Some(&i) => {
                    let prev = &list[i];
                    let changed = ["body", "read", "simNumber", "carrier", "slot", "subId"]
                        .iter()
                        .any(|key| prev.get(*key) != row.get(*key));
                    if changed {
                        if let (Some(obj), Some(src)) = (list[i].as_object_mut(), row.as_object()) {
                            for key in ["body", "read", "simNumber", "slot", "subId", "carrier"] {
                                if let Some(v) = src.get(key) {
                                    obj.insert(key.to_string(), v.clone());
                                }
                            }
                        }
                        updated += 1;
                    }
                }
            }
        }

        list.sort_by(|a, b| {
            let da = a.get("date").and_then(|v| v.as_f64()).unwrap_or(f64::NAN);
            let db = b.get("date").and_then(|v| v.as_f64()).unwrap_or(f64::NAN);
            db.partial_cmp(&da).unwrap_or(std::cmp::Ordering::Equal)
        });

        let total = list.len() as u64;
        self.flush_db();
        UpsertResult {
            added,
            updated,
            total,
        }
    }

    pub fn messages_of(&self, phone_id: &str) -> Vec<Value> {
        self.db
            .get("messages")
            .and_then(|v| v.get(phone_id))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
    }

    pub fn message_count(&self, phone_id: &str) -> usize {
        self.db
            .get("messages")
            .and_then(|v| v.get(phone_id))
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0)
    }

    /// Local read marker; the phone's own `read` column never changes because
    /// this client only ever reads.
    pub fn mark_seen(&mut self, address: &str) -> u64 {
        let mut n = 0u64;
        if let Some(messages) = self.db.get_mut("messages").and_then(|v| v.as_object_mut()) {
            for list in messages.values_mut() {
                if let Some(rows) = list.as_array_mut() {
                    for m in rows.iter_mut() {
                        let same = m.get("address").and_then(|v| v.as_str()) == Some(address);
                        let unseen = !matches!(m.get("seen"), Some(Value::Bool(true)));
                        if same && unseen {
                            if let Some(obj) = m.as_object_mut() {
                                obj.insert("seen".into(), json!(true));
                            }
                            n += 1;
                        }
                    }
                }
            }
        }
        if n > 0 {
            self.flush_db();
        }
        n
    }

    pub fn all_messages(&self) -> Vec<Value> {
        let mut out = Vec::new();
        if let Some(messages) = self.db.get("messages").and_then(|v| v.as_object()) {
            for (phone_id, list) in messages {
                if let Some(rows) = list.as_array() {
                    for m in rows {
                        let mut row = m.clone();
                        if let Some(obj) = row.as_object_mut() {
                            obj.insert("phoneId".into(), json!(phone_id));
                        }
                        out.push(row);
                    }
                }
            }
        }
        out.sort_by(|a, b| {
            let da = a.get("date").and_then(|v| v.as_f64()).unwrap_or(f64::NAN);
            let db = b.get("date").and_then(|v| v.as_f64()).unwrap_or(f64::NAN);
            db.partial_cmp(&da).unwrap_or(std::cmp::Ordering::Equal)
        });
        out
    }

    pub fn stats(&self) -> (u64, u64) {
        let mut total = 0u64;
        let mut phones = 0u64;
        if let Some(messages) = self.db.get("messages").and_then(|v| v.as_object()) {
            phones = messages.len() as u64;
            for list in messages.values() {
                if let Some(rows) = list.as_array() {
                    total += rows.len() as u64;
                }
            }
        }
        (total, phones)
    }

    pub fn prune(&mut self) -> u64 {
        let keep_days = self.setting_num("keepDays", 0.0);
        if keep_days <= 0.0 {
            return 0;
        }
        let cutoff = now_ms() as f64 - keep_days * 86_400_000.0;
        let mut removed = 0u64;
        if let Some(messages) = self.db.get_mut("messages").and_then(|v| v.as_object_mut()) {
            for list in messages.values_mut() {
                if let Some(rows) = list.as_array_mut() {
                    let before = rows.len();
                    rows.retain(|m| {
                        m.get("date")
                            .and_then(|v| v.as_f64())
                            .map(|d| d >= cutoff)
                            .unwrap_or(false)
                    });
                    removed += (before - rows.len()) as u64;
                }
            }
        }
        if removed > 0 {
            self.flush_db();
        }
        removed
    }

    pub fn clear_messages(&mut self, phone_id: Option<&str>) {
        if let Some(messages) = self.db.get_mut("messages").and_then(|v| v.as_object_mut()) {
            match phone_id {
                Some(id) => {
                    messages.remove(id);
                }
                None => messages.clear(),
            }
        }
        self.flush_db();
    }

    // Raw access for the sync engine.
    pub fn phone_raw(&self, id: &str) -> Option<Value> {
        self.get_phone(id).cloned()
    }
}

fn message_id(v: &Value) -> Option<i64> {
    v.get("id")
        .and_then(|id| id.as_i64().or_else(|| id.as_u64().map(|u| u as i64)))
}

/// The renderer only ever sees this shape (`publicPhone` in store.js).
pub fn public_phone(p: &Value) -> Value {
    let get = |k: &str| p.get(k).cloned().unwrap_or(Value::Null);
    let num = |k: &str| match p.get(k) {
        Some(Value::Number(n)) => Value::Number(n.clone()),
        _ => json!(0),
    };
    let auth = p.get("authError").cloned().unwrap_or(Value::Bool(false));
    let auth_bool = match auth {
        Value::Bool(b) => b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        Value::String(s) => !s.is_empty(),
        _ => false,
    };
    let has_token = ["tokenEnc", "tokenRaw"]
        .iter()
        .any(|k| p.get(*k).and_then(|v| v.as_str()).map(|s| !s.is_empty()).unwrap_or(false));

    let mut out = Map::new();
    out.insert("id".into(), get("id"));
    out.insert("name".into(), get("name"));
    out.insert("url".into(), get("url"));
    out.insert("clientName".into(), str_or(p, "clientName"));
    out.insert("createdAt".into(), num("createdAt"));
    out.insert("lastSyncAt".into(), num("lastSyncAt"));
    out.insert("lastError".into(), str_or(p, "lastError"));
    out.insert("authError".into(), json!(auth_bool));
    out.insert(
        "health".into(),
        match p.get("health") {
            Some(v) if !v.is_null() => v.clone(),
            _ => Value::Null,
        },
    );
    out.insert("hasToken".into(), json!(has_token));
    out.insert("certFp".into(), str_or(p, "certFp"));
    Value::Object(out)
}

fn str_or(v: &Value, key: &str) -> Value {
    match v.get(key) {
        Some(Value::String(s)) => json!(s),
        _ => json!(""),
    }
}

pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn uuid_v4() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("smsbridge-store-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn defaults_match_store_js() {
        let dir = temp_dir("defaults");
        let store = Store::new(&dir);
        let s = store.settings();
        let keys: Vec<&str> = s.keys().map(|k| k.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "autoStart",
                "startHidden",
                "syncIntervalMin",
                "syncOnStart",
                "notifyNew",
                "pullDays",
                "keepDays",
                "closeToTray",
                "theme",
                "uiScale",
                "lang"
            ]
        );
        assert_eq!(s["uiScale"], json!(1.1));
        assert!(!store.config_path().exists(), "loading must not rewrite a fresh config");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn roundtrips_an_existing_electron_profile() {
        let dir = temp_dir("compat");
        // A config as written by 2.1.0: unknown keys and a plaintext token.
        let cfg = json!({
            "version": 1,
            "settings": { "theme": "light", "uiScale": 1.25, "custom": "kept" },
            "phones": [{
                "id": "p-1",
                "name": "gt5",
                "url": "http://192.168.1.5:8787",
                "clientId": "cid",
                "clientName": "",
                "createdAt": 1,
                "lastSyncAt": 2,
                "lastSeq": 0,
                "lastSmsId": 243,
                "certPem": "PEM",
                "certFp": "fp",
                "tokenRaw": "token-xyz",
                "futureField": 7
            }]
        });
        std::fs::write(
            dir.join("config.json"),
            serde_json::to_string_pretty(&cfg).unwrap(),
        )
        .unwrap();
        let db = json!({ "version": 1, "messages": { "p-1": [
            { "id": 5, "address": "10086", "body": "hi", "date": 10, "type": "inbox", "slot": 0, "subId": 1, "carrier": "电信", "simNumber": "+86", "read": true }
        ]}});
        std::fs::write(dir.join("messages.json"), serde_json::to_string(&db).unwrap()).unwrap();

        let store = Store::new(&dir);
        let phones = store.list_phones();
        assert_eq!(phones.len(), 1);
        assert_eq!(phones[0]["url"], json!("https://192.168.1.5:8787"));
        assert_eq!(phones[0]["hasToken"], json!(true));
        assert_eq!(phones[0]["certFp"], json!("fp"));
        assert_eq!(store.token_of("p-1"), "token-xyz");
        assert_eq!(store.message_count("p-1"), 1);
        assert_eq!(store.settings()["custom"], json!("kept"));
        assert_eq!(store.setting_num("uiScale", 1.0), 1.25);
        // token migrated to tokenEnc, unknown phone key kept
        let raw = store.get_phone("p-1").unwrap();
        assert_eq!(raw["tokenRaw"], json!(""));
        assert!(raw["tokenEnc"].as_str().unwrap().len() > 10);
        assert_eq!(raw["futureField"], json!(7));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn upsert_dedupes_by_id_and_updates_in_place() {
        let dir = temp_dir("upsert");
        let mut store = Store::new(&dir);
        let rows = vec![
            json!({ "id": 1, "address": "a", "body": "x", "date": 100, "type": "inbox", "slot": 0, "subId": 1, "carrier": "", "simNumber": "", "read": false }),
            json!({ "id": 2, "address": "b", "body": "y", "date": 200, "type": "inbox", "slot": -1, "subId": 0, "carrier": "", "simNumber": "", "read": false }),
        ];
        let first = store.upsert_messages("p", &rows);
        assert_eq!((first.added, first.updated, first.total), (2, 0, 2));
        assert_eq!(store.messages_of("p")[0]["id"], json!(2), "newest first");
        let second = store.upsert_messages(
            "p",
            &[json!({ "id": 1, "address": "a", "body": "x2", "date": 100, "type": "inbox", "slot": 0, "subId": 1, "carrier": "", "simNumber": "", "read": true })],
        );
        assert_eq!((second.added, second.updated, second.total), (0, 1, 2));
        let updated = store
            .messages_of("p")
            .into_iter()
            .find(|m| m["id"] == json!(1))
            .unwrap();
        assert_eq!(updated["body"], json!("x2"));
        assert_eq!(updated["read"], json!(true));
        assert_eq!(store.mark_seen("a"), 1);
        assert_eq!(store.mark_seen("a"), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

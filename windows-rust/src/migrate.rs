//! Profile recovery across the rename "SMS Glass" -> "Windows SMS Bridge".
//!
//! Ported from the Electron build's `src/core/migrate.js`, with the one bug in
//! that port fixed (see below). The merge rules are unchanged:
//!   · de-duplicate by phone id and by message id, so running twice is harmless
//!   · keep the target's settings (theme, autostart) intact
//!   · never lose a pairing: the only copy of the DPAPI-sealed token lives here
//!
//! **Fixed bug:** `migrate.js` runs on every start and re-merges anything
//! missing from the target profile, so a phone the user deleted came back on
//! the next launch (and "clear local messages" was undone too). Two guards now
//! prevent that, both stored in `config.json` (old clients preserve unknown
//! root keys, so the file stays compatible):
//!   · `legacyMigrated` — each legacy profile is recovered at most once. The
//!     merge only happens when the target profile is missing or has no phones
//!     yet (the "early build created an empty profile" case this exists for);
//!     a target that already has phones is authoritative.
//!   · `removedPhoneIds` — deletions are remembered, so even a re-run can never
//!     resurrect that device or its cached messages.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

const LEGACY_NAMES: &[&str] = &["SMS Glass"];

#[derive(Debug, Default, Clone)]
pub struct MigrateResult {
    pub phones: u64,
    pub messages: u64,
    pub from: String,
    /// Legacy profiles skipped because the target profile is authoritative.
    pub skipped: Vec<String>,
}

fn read_json(file: &Path) -> Option<Value> {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
}

fn phone_ids(phones: &[Value]) -> HashSet<String> {
    phones
        .iter()
        .filter_map(|p| p.get("id").and_then(|v| v.as_str()).map(str::to_string))
        .collect()
}

fn string_set(cfg: &Value, key: &str) -> HashSet<String> {
    cfg.get(key)
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn migrated_names(cfg: &Value) -> HashSet<String> {
    cfg.get("legacyMigrated")
        .and_then(|v| v.as_object())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}

pub fn recover_profile(app_data: &Path, app_name: &str) -> MigrateResult {
    recover_profile_with(app_data, app_name, LEGACY_NAMES)
}

pub fn recover_profile_with(
    app_data: &Path,
    app_name: &str,
    legacy_names: &[&str],
) -> MigrateResult {
    let mut result = MigrateResult::default();
    let target = app_data.join(app_name);
    let config_path = target.join("config.json");
    let mut cfg: Option<Value> = read_json(&config_path);
    let mut dirty = false;

    // A profile that already holds pairings is authoritative: do not merge an
    // old profile into it (that is what resurrected deleted phones).
    let target_populated = cfg
        .as_ref()
        .and_then(|c| c.get("phones"))
        .and_then(|v| v.as_array())
        .map(|list| !list.is_empty())
        .unwrap_or(false);

    for legacy_name in legacy_names {
        if cfg
            .as_ref()
            .map(|c| migrated_names(c).contains(*legacy_name))
            .unwrap_or(false)
        {
            continue;
        }

        let legacy = app_data.join(legacy_name);
        let Some(src_cfg) = read_json(&legacy.join("config.json")) else {
            continue;
        };
        let src_phones = src_cfg
            .get("phones")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if src_phones.is_empty() {
            continue;
        }

        if target_populated {
            // Already migrated by an earlier build, or the user has since
            // deleted devices: keep the target as-is and only remember that
            // this legacy profile was handled.
            result.skipped.push((*legacy_name).to_string());
            mark_migrated(&mut cfg, *legacy_name);
            dirty = true;            println!(
                "[migrate] {legacy_name} profile ignored: target already has phones"
            );
            continue;
        }

        let _ = std::fs::create_dir_all(&target);

        if cfg.is_none() {
            if std::fs::copy(legacy.join("config.json"), &config_path).is_ok() {
                cfg = read_json(&config_path);
                result.phones += src_phones.len() as u64;
                println!("[migrate] profile copied from {}", legacy.display());
            }
        }

        if let Some(current) = cfg.as_mut() {
            let removed = string_set(current, "removedPhoneIds");
            let have: HashSet<String> = current
                .get("phones")
                .and_then(|v| v.as_array())
                .map(|list| phone_ids(list))
                .unwrap_or_default();
            let missing: Vec<Value> = src_phones
                .iter()
                .filter(|p| {
                    p.get("id")
                        .and_then(|v| v.as_str())
                        .map(|id| !have.contains(id) && !removed.contains(id))
                        .unwrap_or(false)
                })
                .cloned()
                .collect();
            if !missing.is_empty() {
                let count = missing.len() as u64;
                let root = current.as_object_mut().expect("config object");
                match root.get_mut("phones").and_then(|v| v.as_array_mut()) {
                    Some(list) => list.extend(missing),
                    None => {
                        root.insert("phones".into(), Value::Array(missing));
                    }
                }
                result.phones += count;
                let _ = &mut dirty; // the marker write below flips it
                println!("[migrate] recovered {count} pairing(s) from {legacy_name}");
            }
        }

        // cached messages: union per phone, de-duplicated by the phone's own _id
        merge_messages(&legacy, &target, &cfg, &mut result);

        mark_migrated(&mut cfg, *legacy_name);
        dirty = true;
        if result.from.is_empty() {
            result.from = (*legacy_name).to_string();
        }
    }

    if dirty {
        if let Some(cfg) = cfg.as_ref() {
            let text = serde_json::to_string_pretty(cfg).unwrap_or_default();
            let _ = std::fs::write(&config_path, text);
        }
    }

    result
}

fn mark_migrated(cfg: &mut Option<Value>, legacy_name: &str) {
    let Some(cfg) = cfg.as_mut() else { return };
    if !cfg.is_object() {
        *cfg = json!({});
    }
    let root = cfg.as_object_mut().unwrap();
    if !matches!(root.get("legacyMigrated"), Some(Value::Object(_))) {
        root.insert("legacyMigrated".into(), json!({}));
    }
    if let Some(map) = root.get_mut("legacyMigrated").and_then(|v| v.as_object_mut()) {
        map.insert(legacy_name.to_string(), json!(crate::store::now_ms()));
    }
}

fn merge_messages(legacy: &Path, target: &Path, cfg: &Option<Value>, result: &mut MigrateResult) {
    let Some(src_msgs) = read_json(&legacy.join("messages.json")) else {
        return;
    };
    let Some(src_map) = src_msgs.get("messages").and_then(|v| v.as_object()) else {
        return;
    };
    let removed = cfg.as_ref().map(|c| string_set(c, "removedPhoneIds")).unwrap_or_default();
    let dst_msgs_path = target.join("messages.json");
    let mut dst_msgs = read_json(&dst_msgs_path).unwrap_or_else(|| json!({ "version": 1, "messages": {} }));
    if !dst_msgs.is_object() {
        dst_msgs = json!({ "version": 1, "messages": {} });
    }
    {
        let root = dst_msgs.as_object_mut().unwrap();
        if !matches!(root.get("messages"), Some(Value::Object(_))) {
            root.insert("messages".into(), json!({}));
        }
    }

    let mut added = 0u64;
    let dst_map = dst_msgs
        .get_mut("messages")
        .and_then(|v| v.as_object_mut())
        .unwrap();
    for (phone_id, list) in src_map {
        if removed.contains(phone_id) {
            continue;
        }
        let Some(src_rows) = list.as_array() else {
            continue;
        };
        let current = dst_map
            .get(phone_id)
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let ids: Vec<i64> = current.iter().filter_map(id_of).collect();
        let extra: Vec<Value> = src_rows
            .iter()
            .filter(|m| id_of(m).map(|id| !ids.contains(&id)).unwrap_or(true))
            .cloned()
            .collect();
        if extra.is_empty() {
            continue;
        }
        let mut merged = current;
        merged.extend(extra.iter().cloned());
        merged.sort_by(|a, b| id_of(a).unwrap_or(0).cmp(&id_of(b).unwrap_or(0)));
        dst_map.insert(phone_id.clone(), Value::Array(merged));
        added += extra.len() as u64;
    }

    if added > 0 {
        let text = serde_json::to_string(&dst_msgs).unwrap_or_default();
        let _ = std::fs::write(&dst_msgs_path, text);
        result.messages += added;
        println!("[migrate] recovered {added} cached message(s)");
    }
}

fn id_of(v: &Value) -> Option<i64> {
    v.get("id")
        .and_then(|id| id.as_i64().or_else(|| id.as_u64().map(|u| u as i64)))
}

/// `%APPDATA%` — Electron's `app.getPath('appData')`.
pub fn app_data_root() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("smsbridge-migrate-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_legacy(root: &Path, phones: Value, messages: Option<Value>) {
        let legacy = root.join("SMS Glass");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(
            legacy.join("config.json"),
            serde_json::to_string_pretty(&json!({
                "version": 1,
                "settings": { "theme": "light" },
                "phones": phones
            }))
            .unwrap(),
        )
        .unwrap();
        if let Some(messages) = messages {
            std::fs::write(
                legacy.join("messages.json"),
                serde_json::to_string(&messages).unwrap(),
            )
            .unwrap();
        }
    }

    #[test]
    fn copies_legacy_profile_once() {
        let root = temp_dir("copy");
        write_legacy(
            &root,
            json!([{ "id": "p1", "name": "old" }]),
            Some(json!({ "version": 1, "messages": { "p1": [
                { "id": 2, "address": "a", "body": "b2", "date": 2 },
                { "id": 1, "address": "a", "body": "b1", "date": 1 }
            ]}})),
        );

        let first = recover_profile(&root, "Windows SMS Bridge");
        assert_eq!(first.phones, 1);
        assert_eq!(first.messages, 2);
        assert_eq!(first.from, "SMS Glass");

        // second run is a no-op (marker written)
        let second = recover_profile(&root, "Windows SMS Bridge");
        assert_eq!(second.phones, 0);
        assert_eq!(second.messages, 0);

        let merged: Value = serde_json::from_str(
            &std::fs::read_to_string(root.join("Windows SMS Bridge").join("messages.json")).unwrap(),
        )
        .unwrap();
        let rows = merged["messages"]["p1"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["id"], json!(1), "migrate sorts ascending like migrate.js");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The bug the user hit: delete a paired phone, restart, and it is back.
    #[test]
    fn deleted_phone_does_not_come_back() {
        let root = temp_dir("resurrect");
        write_legacy(
            &root,
            json!([{ "id": "p1", "name": "old" }, { "id": "p2", "name": "other" }]),
            Some(json!({ "version": 1, "messages": { "p1": [
                { "id": 1, "address": "a", "body": "b1", "date": 1 }
            ], "p2": [
                { "id": 1, "address": "b", "body": "c1", "date": 1 }
            ]}})),
        );

        // first launch migrates everything
        let first = recover_profile(&root, "Windows SMS Bridge");
        assert_eq!(first.phones, 2);

        // user deletes p1 in the app: config keeps a tombstone, messages drop
        let config_path = root.join("Windows SMS Bridge").join("config.json");
        let mut cfg: Value =
            serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        cfg["phones"] = json!([{ "id": "p2", "name": "other" }]);
        cfg["removedPhoneIds"] = json!(["p1"]);
        std::fs::write(&config_path, serde_json::to_string_pretty(&cfg).unwrap()).unwrap();
        std::fs::write(
            root.join("Windows SMS Bridge").join("messages.json"),
            serde_json::to_string(&json!({ "version": 1, "messages": {
                "p2": [{ "id": 1, "address": "b", "body": "c1", "date": 1 }]
            }}))
            .unwrap(),
        )
        .unwrap();

        // restart → the deleted phone must stay deleted, its messages too
        let second = recover_profile(&root, "Windows SMS Bridge");
        assert_eq!(second.phones, 0, "deleted phone must not be re-merged");
        assert_eq!(second.messages, 0, "deleted phone's messages must not be re-merged");
        let cfg: Value =
            serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(cfg["phones"].as_array().unwrap().len(), 1);
        assert_eq!(cfg["phones"][0]["id"], json!("p2"));
        let messages: Value = serde_json::from_str(
            &std::fs::read_to_string(root.join("Windows SMS Bridge").join("messages.json")).unwrap(),
        )
        .unwrap();
        assert!(messages["messages"].get("p1").is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A profile that already has pairings is authoritative — no merge at all.
    #[test]
    fn populated_target_is_authoritative() {
        let root = temp_dir("authoritative");
        write_legacy(&root, json!([{ "id": "p1" }, { "id": "p2" }]), None);
        let target = root.join("Windows SMS Bridge");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(
            target.join("config.json"),
            serde_json::to_string_pretty(&json!({
                "version": 1,
                "settings": { "theme": "dark" },
                "phones": [{ "id": "p1" }]
            }))
            .unwrap(),
        )
        .unwrap();

        let result = recover_profile(&root, "Windows SMS Bridge");
        assert_eq!(result.phones, 0);
        assert_eq!(result.skipped, vec!["SMS Glass".to_string()]);
        let cfg: Value =
            serde_json::from_str(&std::fs::read_to_string(target.join("config.json")).unwrap()).unwrap();
        assert_eq!(cfg["phones"].as_array().unwrap().len(), 1);
        assert_eq!(cfg["settings"]["theme"], json!("dark"), "target settings kept");
        assert!(cfg["legacyMigrated"]["SMS Glass"].is_number());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The case the recovery exists for: an early build created an empty profile.
    #[test]
    fn empty_target_profile_is_filled_from_legacy() {
        let root = temp_dir("empty-target");
        write_legacy(&root, json!([{ "id": "p1" }, { "id": "p2" }]), None);
        let target = root.join("Windows SMS Bridge");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(
            target.join("config.json"),
            serde_json::to_string_pretty(&json!({ "version": 1, "settings": {}, "phones": [] }))
                .unwrap(),
        )
        .unwrap();

        let result = recover_profile(&root, "Windows SMS Bridge");
        assert_eq!(result.phones, 2);
        let cfg: Value =
            serde_json::from_str(&std::fs::read_to_string(target.join("config.json")).unwrap()).unwrap();
        assert_eq!(cfg["phones"].as_array().unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(&root);
    }
}

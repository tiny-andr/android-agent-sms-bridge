//! CSV export, ported 1:1 from the Electron main process
//! (`windows/main.js`, IPC handler `messages:export`).
//!
//! Contract, byte for byte:
//!   · save dialog title `导出短信` (a hardcoded literal in the old main.js, it
//!     was never run through `t()`, so it stays Chinese in every UI language),
//!     default name `sms-export-<Date.now()>.csv` in the Documents folder,
//!     filter `CSV (*.csv)`; cancel → `{"saved": false}`;
//!   · header line `时间,号码,方向,卡槽,运营商,内容`, translated through the shared
//!     dictionary and **not** quoted;
//!   · one body line per message, 6 fields, all double-quoted with inner `"`
//!     doubled; direction is `收` for `type == "inbox"` and `发` for anything
//!     else; rows are `store.messagesOf(phoneId)` or `store.allMessages()`
//!     (already newest-first);
//!   · UTF-8 **with BOM**, lines joined with `\r\n`, no trailing newline,
//!     written with `fs.writeFileSync(filePath, ..., 'utf8')`;
//!   · `{"saved": true, "filePath": <path>, "count": <data rows>}`.
//!
//! # Timestamp column
//!
//! The old code formatted the date with
//! `new Date(m.date).toLocaleString(locale)` where `locale` is `zh-CN` for UI
//! language `zh`, `ja-JP` for `ja`, `en-US` otherwise. Those are **Node ICU**
//! strings, not something `strftime` does, so the differences below are the
//! real contract (`format_timestamp` reproduces them and
//! `scripts/gen-csv-fixture.cjs` + `scripts/csv-fixture.json` pin them):
//!
//! | lang  | Node `toLocaleString` pattern              | measured sample                     |
//! |-------|--------------------------------------------|-------------------------------------|
//! | `zh`  | `yyyy/M/d HH:mm:ss` — 24h, **hour padded** | `2026/11/4 00:05:00`, `2024/1/2 09:03:04` |
//! | `ja`  | `y/M/d H:mm:ss` — 24h, **hour bare**       | `2026/11/4 0:05:00`, `2024/1/2 9:03:04`   |
//! | `en`  | `M/d/yyyy, h:mm:ss A` — 12h, hour bare, comma | `11/4/2026, 10:46:40 PM`, `1/2/2024, 9:03:04 AM` |
//!
//! Midnight is `00:00:00` (zh) / `0:00:00` (ja) / `12:00:00 AM` (en); noon is
//! `12:00:00 PM`; seconds are always two digits; sub-second milliseconds are
//! dropped. The value is rendered in the machine's local timezone
//! (`chrono::Local`), which is what a browser process does too.
//!
//! The dialog is the only part that cannot be unit-tested; everything else is
//! pure and covered by the tests at the bottom of this file.
//!
//! The save dialog is a native blocking dialog: the command layer calls
//! `export_messages` from `tauri::async_runtime::spawn_blocking`, never from the
//! UI thread (the dialog plugin's `blocking_save_file` would freeze the window
//! there — it only deadlocks when called from a *non-main* thread that then
//! blocks the main thread, which is why the command wrapper, not this function,
//! owns the background thread).

use chrono::TimeZone;

use serde_json::{json, Value};

use crate::i18n;

/// UTF-8 byte order mark the old writer prepended (`'\ufeff' + ...`).
const BOM: char = '\u{feff}';

/// Chinese keys of the CSV header, in the order `toLocaleString` used them.
const HEADER_KEYS: [&str; 6] = ["时间", "号码", "方向", "卡槽", "运营商", "内容"];

/// `dialog.showSaveDialog(win, { title: '导出短信', ... })` — hardcoded, untranslated.
const DIALOG_TITLE: &str = "导出短信";
/// `filters: [{ name: 'CSV', extensions: ['csv'] }]`.
const CSV_FILTER_NAME: &str = "CSV";

// ---------------------------------------------------------------- formatting

/// Node's `Date#toLocaleString` for the three locales the old client used.
///
/// `lang` is the app language id (`zh` / `ja` / anything else → `en-US`),
/// matching `i18n.getLang()`'s ternary in `main.js`.
pub fn format_timestamp(ms: i64, lang: &str) -> String {
    use chrono::{Datelike, Timelike};

    let Some(dt) = chrono::Local.timestamp_millis_opt(ms).single() else {
        // `new Date(NaN).toLocaleString()` / out-of-range Date → "Invalid Date".
        return "Invalid Date".to_string();
    };

    let (year, month, day) = (dt.year(), dt.month(), dt.day());
    let (hour, minute, second) = (dt.hour(), dt.minute(), dt.second());

    match lang {
        "zh" => format!("{year}/{month}/{day} {hour:02}:{minute:02}:{second:02}"),
        "ja" => format!("{year}/{month}/{day} {hour}:{minute:02}:{second:02}"),
        // `en-US`: `M/d/yyyy, h:mm:ss AM/PM` — 12h clock, bare hour, comma
        // after the date, and 12 (not 0) for both midnight and noon.
        _ => {
            let hour12 = match hour {
                0 => 12,
                h if h > 12 => h - 12,
                h => h,
            };
            let meridiem = if hour < 12 { "AM" } else { "PM" };
            format!("{month}/{day}/{year}, {hour12}:{minute:02}:{second:02} {meridiem}")
        }
    }
}

/// Renders the whole file: BOM + quoted header line + quoted body lines,
/// joined with CRLF, no trailing newline.
///
/// The header and the `收`/`发` direction go through the shared dictionary,
/// which is process-global; the requested `lang` is made current for the call
/// and restored afterwards, so callers get output for `lang` regardless of the
/// UI state.
pub fn render_csv(rows: &[Value], lang: &str) -> String {
    let previous = i18n::get_lang();
    if previous != lang {
        i18n::set_lang(lang);
    }

    let mut lines: Vec<String> = Vec::with_capacity(rows.len() + 1);
    let header: Vec<String> = HEADER_KEYS.iter().map(|key| i18n::ti(key)).collect();
    lines.push(header.join(","));

    for row in rows {
        let timestamp = match date_ms(row) {
            Some(ms) => format_timestamp(ms, lang),
            // `new Date(undefined).toLocaleString()` → "Invalid Date"
            None => "Invalid Date".to_string(),
        };
        let direction = if row.get("type").and_then(Value::as_str) == Some("inbox") {
            i18n::ti("收")
        } else {
            i18n::ti("发")
        };
        let fields = [
            timestamp,
            field(row, "address"),
            direction,
            field(row, "slot"),
            field(row, "carrier"),
            field(row, "body"),
        ];
        lines.push(
            fields
                .iter()
                .map(|value| quote(value))
                .collect::<Vec<_>>()
                .join(","),
        );
    }

    i18n::set_lang(previous);

    let mut out = String::with_capacity(2_048);
    out.push(BOM);
    out.push_str(&lines.join("\r\n"));
    out
}

/// `new Date(m.date)` → epoch milliseconds; `None` when JavaScript would build
/// an invalid Date (missing field, null, NaN, Infinity).
fn date_ms(row: &Value) -> Option<i64> {
    let raw = row.get("date")?.as_f64()?;
    if !raw.is_finite() {
        return None;
    }
    // ECMAScript TimeClip truncates toward zero to integer milliseconds.
    Some(raw.trunc() as i64)
}

/// `esc(v)`: `"${String(v ?? '').replace(/"/g, '""')}"`.
fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        if ch == '"' {
            out.push('"'); // doubled inside the quoted field
        }
        out.push(ch);
    }
    out.push('"');
    out
}

/// `String(row[key] ?? '')` — JS scalar stringification.
fn field(row: &Value, key: &str) -> String {
    match row.get(key) {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Bool(flag)) => flag.to_string(),
        Some(Value::Number(number)) => number_to_string(number),
        // JS would print "[object Object]" / a joined array; the store never
        // carries those on the exported fields.
        Some(other) => other.to_string(),
    }
}

/// `String(number)`: Rust's f64 Display already matches JS for the values that
/// occur here (0, 1, -1), and integers stringify identically.
fn number_to_string(number: &serde_json::Number) -> String {
    match number.as_i64() {
        Some(int) => int.to_string(),
        None => match number.as_u64() {
            Some(uint) => uint.to_string(),
            None => number.as_f64().map(|f| f.to_string()).unwrap_or_default(),
        },
    }
}

// ------------------------------------------------------------------- command

/// `messages:export` — show the save dialog, write the CSV, report the result.
///
/// The command layer (`crate::commands::messages_export`) already wraps this in
/// `spawn_blocking`, so the native dialog never runs on the UI thread.
pub fn export_messages(
    app: &tauri::AppHandle,
    phone_id: Option<&str>,
) -> Result<Value, String> {
    let (rows, lang) = collect(app, phone_id);

    let Some(path) = pick_save_path(app)? else {
        return Ok(json!({ "saved": false }));
    };

    let csv = render_csv(&rows, &lang);
    std::fs::write(&path, csv.as_bytes()).map_err(|err| err.to_string())?;

    Ok(json!({
        "saved": true,
        "filePath": path.to_string_lossy(),
        "count": rows.len(),
    }))
}

/// Snapshot of the messages to export plus the language they were formatted in.
/// The store lock is released before the dialog opens so sync is never blocked
/// by a user staring at a file picker.
fn collect(app: &tauri::AppHandle, phone_id: Option<&str>) -> (Vec<Value>, String) {
    use tauri::Manager;

    let state = app.state::<crate::app::AppState>();
    // A panic elsewhere while the store was locked must not break export.
    let store = state.store.lock().unwrap_or_else(|poison| poison.into_inner());

    let rows = match phone_id {
        Some(id) => store.messages_of(id),
        None => store.all_messages(),
    };
    let setting = store.setting_str("lang", "auto");
    drop(store);

    // In Electron this was `i18n.getLang()`; the setting is what it resolved from.
    let lang = i18n::resolve_lang(&setting, &i18n::system_locale()).to_string();
    (rows, lang)
}

/// Shows the save dialog and returns the chosen path. `None` means the user
/// cancelled. Blocking — the command layer calls it from `spawn_blocking`.
fn pick_save_path(app: &tauri::AppHandle) -> Result<Option<std::path::PathBuf>, String> {
    use tauri::Manager;
    use tauri_plugin_dialog::DialogExt;

    // Electron: `path.join(app.getPath('documents'), 'sms-export-<Date.now()>.csv')`
    let documents = app.path().document_dir().ok();
    let file_name = format!(
        "sms-export-{}.csv",
        chrono::Utc::now().timestamp_millis()
    );

    let mut builder = app
        .dialog()
        .file()
        .set_title(DIALOG_TITLE)
        .set_file_name(file_name)
        .add_filter(CSV_FILTER_NAME, &["csv"]);
    if let Some(dir) = documents {
        builder = builder.set_directory(dir);
    }

    match builder.blocking_save_file() {
        None => Ok(None),
        Some(path) => path
            .into_path()
            .map(Some)
            .map_err(|err| err.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Serialises the tests that touch the process-global UI language with the
    /// i18n tests (cargo runs test cases on separate threads; the dictionary's
    /// current language is one process-wide value, so the whole test binary has
    /// to share a single lock).
    fn lang_guard() -> std::sync::MutexGuard<'static, ()> {
        crate::i18n::test_guard()
    }

    fn fixture() -> Value {
        serde_json::from_str(include_str!("../scripts/csv-fixture.json"))
            .expect("scripts/csv-fixture.json must be valid JSON")
    }

    fn msg(
        date: i64,
        kind: &str,
        address: &str,
        slot: i64,
        carrier: &str,
        body: &str,
    ) -> Value {
        json!({
            "id": 1,
            "date": date,
            "type": kind,
            "address": address,
            "slot": slot,
            "subId": 1,
            "carrier": carrier,
            "body": body,
            "read": true,
        })
    }

    fn without_bom(csv: &str) -> &str {
        csv.strip_prefix(BOM).expect("CSV must start with a UTF-8 BOM")
    }

    fn line<'a>(csv: &'a str, index: usize) -> &'a str {
        without_bom(csv)
            .split("\r\n")
            .nth(index)
            .unwrap_or_else(|| panic!("CSV has no line {index}"))
    }

    // ---------------------------------------------------------- timestamps

    #[test]
    fn timestamps_match_node_icu() {
        let fx = fixture();
        let time_zone = fx["timeZone"].as_str().unwrap_or("unknown");
        let cases = fx["cases"].as_array().expect("fixture.cases");
        assert!(cases.len() >= 15, "fixture should cover all edge cases");

        for case in cases {
            let label = case["label"].as_str().unwrap_or("?");
            let ms = case["ms"].as_i64().unwrap_or_else(|| panic!("{label}: bad ms"));
            let expected_offset = case["utcOffsetMinutes"]
                .as_i64()
                .unwrap_or_else(|| panic!("{label}: bad utcOffsetMinutes"));

            // The fixture's strings are wall-clock output, so they only hold in
            // the timezone they were generated in. Fail loudly rather than
            // silently comparing across timezones.
            let local = chrono::Local
                .timestamp_millis_opt(ms)
                .single()
                .unwrap_or_else(|| panic!("{label}: not a valid local datetime"));
            let local_offset = local.offset().local_minus_utc() as i64 / 60;
            assert_eq!(
                local_offset, expected_offset,
                "fixture was generated in {time_zone} but this machine is offset \
                 {local_offset} min for case {label}; re-run \
                 `node scripts/gen-csv-fixture.cjs` here"
            );

            for lang in ["zh", "ja", "en"] {
                let expected = case["expected"][lang]
                    .as_str()
                    .unwrap_or_else(|| panic!("{label}: no expected.{lang}"));
                assert_eq!(
                    format_timestamp(ms, lang),
                    expected,
                    "case {label} lang {lang} (ms {ms})"
                );
            }
        }
    }

    #[test]
    fn unknown_language_falls_back_to_en_us() {
        let fx = fixture();
        let first = &fx["cases"][0];
        let ms = first["ms"].as_i64().unwrap();
        assert_eq!(
            format_timestamp(ms, "de"),
            first["expected"]["en"].as_str().unwrap()
        );
    }

    // -------------------------------------------------------------- header

    #[test]
    fn header_is_translated_per_language_and_unquoted() {
        let _guard = lang_guard();
        for (lang, expected) in [
            ("zh", "时间,号码,方向,卡槽,运营商,内容"),
            ("en", "Date,Number,Direction,Slot,Carrier,Message"),
            ("ja", "日時,番号,方向,スロット,通信事業者,本文"),
        ] {
            let csv = render_csv(&[], lang);
            assert_eq!(line(&csv, 0), expected, "header for lang {lang}");
            assert!(!line(&csv, 0).contains('"'), "header must not be quoted");
        }
    }

    #[test]
    fn empty_export_is_just_the_header() {
        let _guard = lang_guard();
        assert_eq!(
            render_csv(&[], "en"),
            "\u{feff}Date,Number,Direction,Slot,Carrier,Message"
        );
    }

    // ------------------------------------------------------- row structure

    #[test]
    fn body_lines_quote_every_field_and_direction_is_translated() {
        let _guard = lang_guard();
        let rows = vec![
            msg(0, "inbox", "10086", 0, "电信", "验证码 123456"),
            msg(1_000, "sent", "10010", 1, "", "hi"),
        ];

        let zh = render_csv(&rows, "zh");
        let lines: Vec<&str> = without_bom(&zh).split("\r\n").collect();
        assert_eq!(lines.len(), 3, "header + 2 messages");
        assert!(lines[1].ends_with(",\"10086\",\"收\",\"0\",\"电信\",\"验证码 123456\""));
        assert!(lines[2].ends_with(",\"10010\",\"发\",\"1\",\"\",\"hi\""));

        let en = render_csv(&rows, "en");
        assert!(without_bom(&en).contains("\"In\""));
        assert!(without_bom(&en).contains("\"Out\""));

        let ja = render_csv(&rows, "ja");
        assert!(without_bom(&ja).contains("\"受\""));
        assert!(without_bom(&ja).contains("\"送\""));
    }

    #[test]
    fn inbox_is_incoming_everything_else_outgoing() {
        let _guard = lang_guard();
        for (kind, expected) in [("inbox", "收"), ("sent", "发"), ("unknown", "发"), ("", "发")] {
            let csv = render_csv(&[msg(0, kind, "10086", 0, "", "x")], "zh");
            assert!(
                line(&csv, 1).contains(&format!("\"{expected}\"")),
                "type {kind:?} must render as {expected}"
            );
        }
    }

    #[test]
    fn double_quotes_are_doubled_and_newlines_survive() {
        let _guard = lang_guard();
        let rows = vec![msg(
            0,
            "inbox",
            "10086",
            0,
            "carrier \"X\"",
            "line1\nline2",
        )];
        let csv = render_csv(&rows, "en");
        let body = line(&csv, 1);
        assert!(
            body.contains("\"carrier \"\"X\"\"\""),
            "inner quotes doubled: {body}"
        );
        // The old writer never rewrote newlines, they stay raw inside the quotes.
        assert!(body.ends_with("\"line1\nline2\""), "body field: {body}");
    }

    #[test]
    fn missing_fields_become_empty_quoted_strings() {
        let _guard = lang_guard();
        let csv = render_csv(&[json!({ "date": 0 })], "en");
        let body = line(&csv, 1);
        // timestamp, then "", "Out", "", "", "" — JS `String(v ?? '')`
        assert!(
            body.ends_with(",\"\",\"Out\",\"\",\"\",\"\""),
            "empty fields: {body}"
        );
    }

    #[test]
    fn invalid_date_renders_like_javascript() {
        let _guard = lang_guard();
        let rows = vec![json!({ "body": "x", "type": "inbox" })];
        let csv = render_csv(&rows, "en");
        assert!(line(&csv, 1).starts_with("\"Invalid Date\""));
    }

    // ------------------------------------------------------ file envelope

    #[test]
    fn file_has_bom_crlf_and_no_trailing_newline() {
        let _guard = lang_guard();
        let rows = vec![
            msg(0, "inbox", "a", 0, "", "1"),
            msg(1, "sent", "b", 1, "", "2"),
            msg(2, "inbox", "c", 0, "", "3"),
        ];
        let csv = render_csv(&rows, "en");

        assert!(csv.starts_with('\u{feff}'), "BOM prefix");
        assert_eq!(
            csv.strip_prefix('\u{feff}').unwrap().matches("\r\n").count(),
            3,
            "header + 3 messages → 3 CRLF separators"
        );
        assert!(!csv.ends_with("\r\n"), "no trailing newline");
        assert!(!csv.ends_with('\n'), "no trailing newline");
        // Nothing but the line separators is a CR/LF (bodies here have none).
        let body = csv.strip_prefix('\u{feff}').unwrap().replace("\r\n", "");
        assert!(!body.contains('\r') && !body.contains('\n'));
    }
}

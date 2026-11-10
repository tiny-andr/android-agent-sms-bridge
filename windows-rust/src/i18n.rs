//! Interface language for the native side (tray menu, dialogs, notifications,
//! CSV headers and error messages).
//!
//! The dictionary itself is generated from `windows/src/shared/i18n.js` by
//! `scripts/gen-rust-i18n.cjs` — the renderer and the native side must speak the
//! exact same 258 sentences, so there is one source of truth, not two.
//!
//! Ported 1:1 from the Electron build's `src/shared/i18n.js`:
//! `normalize`, `resolveLang`, `setLang`, `getLang`, `t`, `fill` and the English
//! singular variants (`EN_ONE`).

#[path = "i18n_table.rs"]
mod table;

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

pub use table::OneSpec;

static CURRENT: RwLock<&'static str> = RwLock::new("en");

fn en_map() -> &'static HashMap<&'static str, &'static str> {
    static MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| table::EN.iter().copied().collect())
}

fn ja_map() -> &'static HashMap<&'static str, &'static str> {
    static MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| table::JA.iter().copied().collect())
}

/// `zh*` → zh, `ja*` → ja, anything else → "" (i18n.js `normalize`).
pub fn normalize(value: &str) -> &'static str {
    let v = value.to_lowercase();
    if v.starts_with("zh") {
        "zh"
    } else if v.starts_with("ja") {
        "ja"
    } else if v.starts_with("en") {
        "en"
    } else {
        ""
    }
}

/// 'auto' → follow the system locale, anything else is taken literally.
pub fn resolve_lang(setting: &str, system_locale: &str) -> &'static str {
    let forced = normalize(setting);
    if !forced.is_empty() {
        return forced;
    }
    let sys = normalize(system_locale);
    if sys.is_empty() {
        "en"
    } else {
        sys
    }
}

pub fn set_lang(lang: &str) -> &'static str {
    let next = if matches!(lang, "zh" | "en" | "ja") { lang } else { "en" };
    let mut cur = CURRENT.write().unwrap();
    *cur = match next {
        "zh" => "zh",
        "ja" => "ja",
        _ => "en",
    };
    *cur
}

pub fn get_lang() -> &'static str {
    *CURRENT.read().unwrap()
}

/// `t(key, vars)` — Chinese is the key itself, EN is the fallback table.
pub fn t(key: &str, vars: &[(&str, &str)]) -> String {
    if get_lang() == "zh" {
        return fill(key, vars);
    }
    let text = if get_lang() == "ja" {
        ja_map().get(key).copied().or_else(|| en_map().get(key).copied())
    } else {
        en_map().get(key).copied()
    }
    .unwrap_or(key);

    if get_lang() == "en" && !vars.is_empty() && (var_is_one(vars, "n") || var_is_one(vars, "m")) {
        if let Some(one) = table::EN_ONE.iter().find(|(k, _)| *k == key) {
            match one.1 {
                OneSpec::Text(v) => return fill(v, vars),
                OneSpec::Fn => {
                    if let Some(rendered) = one_fn(key, vars) {
                        return fill(&rendered, vars);
                    }
                }
            }
        }
    }
    fill(text, vars)
}

/// Convenience for sentences without placeholders.
pub fn ti(key: &str) -> String {
    t(key, &[])
}

fn var_is_one(vars: &[(&str, &str)], name: &str) -> bool {
    // JS compares `vars.n === 1`; callers pass numbers through, so a strict "1"
    // is the faithful equivalent.
    vars.iter().any(|(k, v)| *k == name && *v == "1")
}

/// The two EN_ONE entries that carry two independent counts (i18n.js uses
/// arrow functions for them, so they cannot live in the generated table).
fn one_fn(key: &str, vars: &[(&str, &str)]) -> Option<String> {
    let get = |name: &str| -> String {
        vars.iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| (*v).to_string())
            .unwrap_or_default()
    };
    match key {
        "{n} 条 · 最后一条 {when}" => {
            let n = get("n");
            let when = get("when");
            let noun = if n == "1" { "message" } else { "messages" };
            Some(format!("{n} {noun} · last {when}"))
        }
        "{n} 台手机 · {m} 条短信" => {
            let n = get("n");
            let m = get("m");
            let phones = if n == "1" { "phone" } else { "phones" };
            let msgs = if m == "1" { "message" } else { "messages" };
            Some(format!("{n} {phones} · {m} {msgs}"))
        }
        _ => None,
    }
}

/// `{name}` placeholder substitution.
fn fill(text: &str, vars: &[(&str, &str)]) -> String {
    if vars.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            if let Some(close) = text[i + 1..].find('}') {
                let name = &text[i + 1..i + 1 + close];
                let valid = !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_');
                if valid {
                    if let Some((_, v)) = vars.iter().find(|(k, _)| *k == name) {
                        out.push_str(v);
                    } else {
                        out.push_str(&text[i..i + 1 + close + 1]);
                    }
                    i += 1 + close + 1;
                    continue;
                }
            }
        }
        let ch = text[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Electron's `app.getLocale()`, e.g. "zh-CN".
pub fn system_locale() -> String {
    #[cfg(windows)]
    {
        use windows::Win32::Globalization::GetUserDefaultLocaleName;
        let mut buf = [0u16; 85];
        let len = unsafe { GetUserDefaultLocaleName(&mut buf) };
        if len > 0 {
            let s = String::from_utf16_lossy(&buf[..(len as usize - 1)]);
            if !s.is_empty() {
                return s;
            }
        }
    }
    std::env::var("LANG").unwrap_or_default()
}

/// Test-only guard: the dictionary language is process-global, so tests in any
/// module that render translated text must serialise themselves against the
/// i18n tests (cargo runs test cases on separate threads by default).
#[cfg(test)]
pub fn test_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|err| err.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_is_the_key() {
        let _guard = test_guard();
        set_lang("zh");
        assert_eq!(t("设备", &[]), "设备");
        assert_eq!(t("已配对手机 · {n}", &[("n", "2")]), "已配对手机 · 2");
    }

    #[test]
    fn english_and_japanese_tables_load() {
        let _guard = test_guard();
        set_lang("en");
        assert_eq!(t("设备", &[]), "Devices");
        set_lang("ja");
        assert_eq!(t("设备", &[]), "端末");
        set_lang("en");
    }

    #[test]
    fn english_singular_variants() {
        let _guard = test_guard();
        set_lang("en");
        assert_eq!(t("{n} 台手机", &[("n", "1")]), "1 phone");
        assert_eq!(t("{n} 台手机", &[("n", "3")]), "3 phones");
        assert_eq!(
            t("{n} 台手机 · {m} 条短信", &[("n", "1"), ("m", "2")]),
            "1 phone · 2 messages"
        );
        set_lang("en");
    }

    #[test]
    fn normalize_and_resolve() {
        let _guard = test_guard();
        assert_eq!(normalize("zh-TW"), "zh");
        assert_eq!(normalize("ja-JP"), "ja");
        assert_eq!(normalize("en-US"), "en");
        assert_eq!(normalize("auto"), "");
        assert_eq!(resolve_lang("auto", "zh-CN"), "zh");
        assert_eq!(resolve_lang("auto", "de-DE"), "en");
        assert_eq!(resolve_lang("en", "zh-CN"), "en");
    }
}

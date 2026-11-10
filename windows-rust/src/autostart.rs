//! Boot autostart via a single per-user `Run` registry value.
//!
//! The Electron build managed the key itself instead of `app.setLoginItemSettings`
//! because Electron registers that under the AppUserModelId, which is easy to
//! duplicate and impossible to read back reliably. One predictable value only —
//! same name, same command line, plus cleanup of the values older builds left
//! behind.

use winreg::enums::{HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE};
use winreg::RegKey;

use crate::app::APP_NAME;

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
/// Values written by older builds / the old app name (main.js deletes them too).
const LEGACY_VALUES: [&str; 2] = ["com.smsglass.app", "SMS Glass"];

/// `"<exe>" "--hidden"` — the app starts in the tray.
pub fn autostart_value() -> String {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    if exe.is_empty() {
        return String::new();
    }
    format!("\"{exe}\" \"--hidden\"")
}

fn run_key() -> Result<RegKey, String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    hkcu.open_subkey_with_flags(RUN_KEY, KEY_QUERY_VALUE | KEY_SET_VALUE)
        .map_err(|err| format!("HKCU\\{RUN_KEY}: {err}"))
}

pub fn apply(enabled: bool) -> Result<(), String> {
    let key = run_key()?;
    if enabled {
        let value = autostart_value();
        if value.is_empty() {
            return Err("cannot resolve the executable path".into());
        }
        key.set_value(APP_NAME, &value)
            .map_err(|err| format!("set {APP_NAME}: {err}"))?;
    } else {
        let _ = key.delete_value(APP_NAME);
    }
    // clean up entries left by older builds / the old app name
    for legacy in LEGACY_VALUES {
        let _ = key.delete_value(legacy);
    }
    Ok(())
}

/// The command line currently registered, if any (used by tests/diagnostics).
pub fn current_value() -> Option<String> {
    let key = run_key().ok()?;
    key.get_value::<String, _>(APP_NAME).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autostart_value_quotes_the_exe_and_passes_hidden() {
        let value = autostart_value();
        if std::env::current_exe().is_err() {
            return;
        }
        assert!(value.starts_with('"'), "{value}");
        assert!(value.ends_with("\" \"--hidden\""), "{value}");
    }

    #[test]
    fn registry_name_matches_the_old_build() {
        // main.js: reg add HKCU\...\Run /v "Windows SMS Bridge"
        assert_eq!(APP_NAME, "Windows SMS Bridge");
        assert_eq!(LEGACY_VALUES.len(), 2);
    }
}

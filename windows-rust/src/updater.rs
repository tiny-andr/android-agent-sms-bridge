//! Update pipeline for the Windows client — 1:1 port of the Electron build's
//! `src/core/updater.js`.
//!
//! The GitHub release is both the version manifest and the trust anchor: its
//! `digest` field carries the SHA-256 of every asset, so a mirror can only ever
//! serve a byte-identical installer. Mirrors are tried in order for the
//! download only; nothing about the version check is delegated to them.
//!
//! Unlike the LAN client (`smsbridge`), the GitHub API and the public mirrors
//! live outside the LAN, so this module's HTTP agent is allowed to pick up the
//! system proxy from the environment (`HTTP_PROXY` / `HTTPS_PROXY` /
//! `ALL_PROXY`) and uses ureq's default trust roots (the bundled webpki-roots,
//! with the `ring` crypto provider that `main` installs process-wide).
//!
//! Behaviour that must not change (see the JS original):
//!   · version check: GitHub `releases/latest`, 8 s, only newer → `Some`
//!   · asset: first name matching `^Windows-SMS-Bridge-Setup-.*\.exe$`
//!   · download: 5 sources in order, 15 s connect timeout, full fall-through,
//!     partial file deleted at the start of every attempt, size + SHA-256
//!     verified before the file is accepted
//!   · install: detached `cmd.exe /c` that waits 2 s, runs the NSIS installer
//!     silently, then reopens the app; the caller then quits the process.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use sha2::{Digest, Sha256};

/// GitHub release API used as the version manifest.
const API: &str = "https://api.github.com/repos/tiny-andr/android-agent-sms-bridge/releases/latest";
/// Asset name pattern: `^Windows-SMS-Bridge-Setup-.*\.exe$` (no regex dependency).
const ASSET_PREFIX: &str = "Windows-SMS-Bridge-Setup-";
const ASSET_SUFFIX: &str = ".exe";
const UA: &str = "Windows-SMS-Bridge-Updater";
/// Version check timeout (the JS `fetchJson` default).
const API_TIMEOUT: Duration = Duration::from_secs(8);
/// Connect timeout for each mirror attempt (the JS `setTimeout(…, 15000)`).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Empty prefix = GitHub direct, the rest are public mainland mirrors.
const SOURCES: [&str; 5] = [
    "",
    "https://ghfast.top/",
    "https://gh-proxy.com/",
    "https://ghproxy.net/",
    "https://ghproxy.site/",
];
const READ_BUF: usize = 64 * 1024;

/// The release asset (installer) discovered by [`check`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateAsset {
    pub name: String,
    pub size: u64,
    /// Lowercase/uppercase hex from the release `digest`, empty when absent.
    pub sha256: String,
    /// `browser_download_url` — mirrors are prepended to this, never replacing it.
    pub url: String,
}

/// A newer release, ready to download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundUpdate {
    pub version: String,
    pub notes: String,
    pub asset: UpdateAsset,
}

/// Dotted numeric compare, missing parts are 0. Returns 1 when `a > b`,
/// -1 when `a < b`, 0 when equal — same as the JS `compareVersions`.
pub fn compare_versions(a: &str, b: &str) -> i32 {
    let pa = version_parts(a);
    let pb = version_parts(b);
    let n = pa.len().max(pb.len());
    for i in 0..n {
        let x = pa.get(i).copied().unwrap_or(0);
        let y = pb.get(i).copied().unwrap_or(0);
        if x != y {
            return if x > y { 1 } else { -1 };
        }
    }
    0
}

fn version_parts(v: &str) -> Vec<i64> {
    v.split('.').map(js_parse_int).collect()
}

/// `parseInt(part, 10) || 0`: trim leading whitespace, optional sign, then the
/// longest run of leading ASCII digits (no digits at all → 0).
fn js_parse_int(part: &str) -> i64 {
    let bytes = part.as_bytes();
    let mut i = 0;
    while i < bytes.len() && (bytes[i] as char).is_ascii_whitespace() {
        i += 1;
    }
    let mut neg = false;
    if i < bytes.len() && (bytes[i] == b'-' || bytes[i] == b'+') {
        neg = bytes[i] == b'-';
        i += 1;
    }
    let start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if start == i {
        return 0;
    }
    let n: i64 = part[start..i].parse().unwrap_or(i64::MAX);
    if neg {
        -n
    } else {
        n
    }
}

/// Equivalent of `/^Windows-SMS-Bridge-Setup-.*\.exe$/` (case-sensitive, `.` in
/// the middle can be empty, `.exe` must be the final suffix).
fn is_installer_asset(name: &str) -> bool {
    match name.strip_prefix(ASSET_PREFIX) {
        Some(rest) => rest.ends_with(ASSET_SUFFIX),
        None => false,
    }
}

/// `digest.startsWith('sha256:') ? digest.slice(7) : ''`.
fn digest_sha256(digest: &str) -> String {
    match digest.strip_prefix("sha256:") {
        Some(hex) => hex.to_string(),
        None => String::new(),
    }
}

/// Agent for the GitHub API: overall 8 s budget, env proxy allowed.
fn api_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .try_proxy_from_env(true)
        .timeout(API_TIMEOUT)
        .build()
}

/// Agent for mirror downloads: 15 s to connect, no overall budget (matching
/// the JS, which only clears its timer once the response headers arrive).
fn download_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .try_proxy_from_env(true)
        .timeout_connect(CONNECT_TIMEOUT)
        .build()
}

fn http_error(err: ureq::Error) -> String {
    match err {
        ureq::Error::Status(code, _) => format!("HTTP {code}"),
        ureq::Error::Transport(t) => t.to_string(),
    }
}

fn source_label(prefix: &str) -> &str {
    if prefix.is_empty() {
        "github"
    } else {
        prefix
            .strip_prefix("https://")
            .unwrap_or(prefix)
            .trim_end_matches('/')
    }
}

/// Query the GitHub release and return `Some` only when it is newer than
/// `current_version` and carries a matching installer asset.
pub fn check(current_version: &str) -> Result<Option<FoundUpdate>, String> {
    let agent = api_agent();
    let response = agent
        .get(API)
        .set("User-Agent", UA)
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(http_error)?;
    let release: serde_json::Value = response.into_json().map_err(|e| e.to_string())?;

    let tag = release
        .get("tag_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    // JS `String(tag_name || '').replace(/^v/, '')`.
    let version = tag.strip_prefix('v').unwrap_or(tag).to_string();
    if version.is_empty() || compare_versions(&version, current_version) <= 0 {
        return Ok(None);
    }

    let empty = Vec::new();
    let assets = release
        .get("assets")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty);
    let Some(found) = assets
        .iter()
        .find(|a| is_installer_asset(a.get("name").and_then(|v| v.as_str()).unwrap_or("")))
    else {
        return Ok(None);
    };

    let asset = UpdateAsset {
        name: found
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        size: found
            .get("size")
            .and_then(|v| v.as_u64())
            .or_else(|| {
                found
                    .get("size")
                    .and_then(|v| v.as_f64())
                    .filter(|n| *n > 0.0)
                    .map(|n| n as u64)
            })
            .unwrap_or(0),
        sha256: digest_sha256(found.get("digest").and_then(|v| v.as_str()).unwrap_or("")),
        url: found
            .get("browser_download_url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    };

    Ok(Some(FoundUpdate {
        version,
        notes: release
            .get("body")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        asset,
    }))
}

/// Download the installer into `dir`, falling through the mirror list on any
/// failure. Returns the path of the verified file.
pub fn download(
    asset: &UpdateAsset,
    dir: &Path,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|e| format!("create_dir_all {}: {e}", dir.display()))?;
    let target = dir.join(&asset.name);
    let agent = download_agent();
    let mut last_error: Option<String> = None;

    for prefix in SOURCES {
        match attempt_download(&agent, prefix, asset, &target, &mut on_progress) {
            Ok(()) => return Ok(target),
            Err(err) => {
                eprintln!("[updater] {} failed: {err}", source_label(prefix));
                last_error = Some(err);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| "download failed".to_string()))
}

fn attempt_download(
    agent: &ureq::Agent,
    prefix: &str,
    asset: &UpdateAsset,
    target: &Path,
    on_progress: &mut impl FnMut(u64, u64),
) -> Result<(), String> {
    // Delete any partial file left by a previous attempt before starting.
    if target.exists() {
        fs::remove_file(target).map_err(|e| format!("remove {}: {e}", target.display()))?;
    }

    let url = format!("{prefix}{}", asset.url);
    let response = agent
        .get(&url)
        .set("User-Agent", UA)
        .call()
        .map_err(http_error)?;

    // total = content-length || asset.size || 0 (JS `Number(...) || asset.size || 0`).
    let total = response
        .header("content-length")
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(asset.size);

    let mut reader = response.into_reader();
    let mut file = File::create(target).map_err(|e| format!("create {}: {e}", target.display()))?;
    let mut buf = vec![0u8; READ_BUF];
    let mut done: u64 = 0;
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => return Err(format!("read {url}: {e}")),
        };
        file.write_all(&buf[..n])
            .map_err(|e| format!("write {}: {e}", target.display()))?;
        done += n as u64;
        on_progress(done, total);
    }
    file.flush()
        .map_err(|e| format!("flush {}: {e}", target.display()))?;
    drop(file);

    let size = fs::metadata(target)
        .map_err(|e| format!("stat {}: {e}", target.display()))?
        .len();
    if asset.size > 0 && size != asset.size {
        return Err(format!("size {size}/{}", asset.size));
    }
    if !asset.sha256.is_empty() {
        let got = sha256_file(target)?;
        if !got.eq_ignore_ascii_case(&asset.sha256) {
            return Err("sha256 mismatch".to_string());
        }
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; READ_BUF];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("read {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Runs the NSIS installer silently, then reopens the app. The installer cannot
/// replace files while we hold them, so it waits for this process to exit first;
/// the caller is expected to quit right after this returns.
pub fn install_and_restart(installer: &Path, exe: &Path) {
    let cmd = format!(
        "timeout /t 2 /nobreak >nul & \"{}\" /S & start \"\" \"{}\"",
        installer.display(),
        exe.display()
    );
    let mut command = Command::new("cmd.exe");
    command.arg("/c");
    // Append the command line verbatim, exactly like the JS `spawn('cmd.exe',
    // ['/c', cmd])`: no re-quoting of the embedded quotes / `&`.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.raw_arg(&cmd);
        // windowsHide: true → CREATE_NO_WINDOW
        command.creation_flags(0x0800_0000);
    }
    #[cfg(not(windows))]
    command.arg(&cmd);

    match command.spawn() {
        Ok(_child) => {
            // Detached: do not wait, do not kill on drop (std never does).
        }
        Err(err) => eprintln!("[updater] failed to start installer: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_versions_dotted_numeric() {
        // Equal when one side is just a prefix with implicit zeros.
        assert_eq!(compare_versions("2.1.0", "2.1"), 0);
        assert_eq!(compare_versions("2.1", "2.1.0"), 0);
        assert_eq!(compare_versions("2.0.0", "2"), 0);
        assert_eq!(compare_versions("2", "2.0.0.0"), 0);
        // Numeric, not lexicographic.
        assert_eq!(compare_versions("2.10.0", "2.9.9"), 1);
        assert_eq!(compare_versions("2.9.9", "2.10.0"), -1);
        assert_eq!(compare_versions("2.1.0.1", "2.1.0"), 1);
        assert_eq!(compare_versions("3", "2.99.99"), 1);
        assert_eq!(compare_versions("2.1.0", "2.1.1"), -1);
        // Missing / junk parts behave like `parseInt(x) || 0`.
        assert_eq!(compare_versions("", "0"), 0);
        assert_eq!(compare_versions("", "0.0.0"), 0);
        assert_eq!(compare_versions("2.x.0", "2.0.0"), 0);
        assert_eq!(compare_versions("v2.1", "2.1"), -1); // 'v' parsed as 0, same as parseInt
        assert_eq!(compare_versions("2.1.0", "2.1.0"), 0);
    }

    #[test]
    fn installer_asset_regex_acceptance() {
        assert!(is_installer_asset("Windows-SMS-Bridge-Setup-2.1.0.exe"));
        assert!(is_installer_asset("Windows-SMS-Bridge-Setup-.exe"));
        assert!(is_installer_asset(
            "Windows-SMS-Bridge-Setup-2.1.0-x64.exe"
        ));
        // Wrong suffix or prefix.
        assert!(!is_installer_asset(
            "Windows-SMS-Bridge-Setup-2.1.0.exe.blockmap"
        ));
        assert!(!is_installer_asset("Windows-SMS-Bridge-Setup-2.1.0.zip"));
        assert!(!is_installer_asset("Windows-SMS-Bridge-Setup-2.1.0.msi"));
        assert!(!is_installer_asset("windows-sms-bridge-setup-2.1.0.exe"));
        assert!(!is_installer_asset("Windows-SMS-Bridge-Setup-2.1.0.EXE"));
        assert!(!is_installer_asset("Windows SMS Bridge_2.1.0_x64-setup.exe"));
        assert!(!is_installer_asset(""));
        assert!(!is_installer_asset("Windows-SMS-Bridge-Setup-"));
    }

    #[test]
    fn digest_parsing() {
        assert_eq!(
            digest_sha256("sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        );
        assert_eq!(digest_sha256("sha256:"), "");
        assert_eq!(digest_sha256("md5:abc"), "");
        assert_eq!(digest_sha256(""), "");
        assert_eq!(digest_sha256("SHA256:abc"), "");
    }

    #[test]
    fn progress_total_prefers_content_length() {
        // The progress total rule is only reachable via HTTP, but the clamping
        // logic is worth pinning down: a zero/missing header must fall back to
        // the release metadata size.
        let size = 1234u64;
        let total_from = |header: Option<&str>| {
            header
                .and_then(|v| v.trim().parse::<u64>().ok())
                .filter(|n| *n > 0)
                .unwrap_or(size)
        };
        assert_eq!(total_from(Some("987")), 987);
        assert_eq!(total_from(Some("0")), size);
        assert_eq!(total_from(Some("")), size);
        assert_eq!(total_from(None), size);
    }
}

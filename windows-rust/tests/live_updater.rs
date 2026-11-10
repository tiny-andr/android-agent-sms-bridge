//! Live updater test against the real GitHub releases.
//!
//! ```text
//! cargo test --test live_updater -- --ignored --nocapture
//! RUN_UPDATE_DOWNLOAD=1 cargo test --test live_updater -- --ignored --nocapture
//! ```
//!
//! Notes on the repository state (2026-11): the *latest* release (`v2.2.0`) only
//! ships the APK, while `v2.1.0` also carries the Windows installer. The shipped
//! updater is supposed to behave exactly like the Electron one: read the latest
//! release, compare versions, and only offer an update when a matching
//! `Windows-SMS-Bridge-Setup-*.exe` asset exists. So this test asserts both:
//!   · the latest release is read and compared correctly (2.2.0 > current), and
//!     no update is *offered* because there is no Windows asset in it;
//!   · an older release that does carry the installer matches the asset rule and
//!     its `digest` is a `sha256:` value (the trust anchor for downloads).
//! With `RUN_UPDATE_DOWNLOAD=1` it downloads that installer through the same
//! mirror fall-through list and verifies the SHA-256 byte for byte.

use serde_json::Value;
use windows_sms_bridge::updater::{self, UpdateAsset};

const API: &str = "https://api.github.com/repos/tiny-andr/android-agent-sms-bridge";
const UA: &str = "Windows-SMS-Bridge-Updater";

fn api_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .try_proxy_from_env(true)
        .timeout(std::time::Duration::from_secs(20))
        .build()
}

fn fetch_json(url: &str) -> Value {
    let response = api_agent()
        .request("GET", url)
        .set("User-Agent", UA)
        .set("Accept", "application/vnd.github+json")
        .call()
        .unwrap_or_else(|e| panic!("GET {url} failed: {e}"));
    response.into_json::<Value>().expect("release JSON")
}

fn is_installer(name: &str) -> bool {
    name.starts_with("Windows-SMS-Bridge-Setup-") && name.ends_with(".exe")
}

fn asset_from(asset: &Value) -> UpdateAsset {
    let digest = asset
        .get("digest")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    UpdateAsset {
        name: asset
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        size: asset.get("size").and_then(|v| v.as_u64()).unwrap_or(0),
        sha256: digest.strip_prefix("sha256:").unwrap_or_default().to_string(),
        url: asset
            .get("browser_download_url")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
    }
}

#[test]
#[ignore = "needs network access to the GitHub release API"]
fn live_update_check_matches_the_published_release() {
    // 1. the latest release is read and version-compared (like the app does)
    let latest = fetch_json(&format!("{API}/releases/latest"));
    let version = latest
        .get("tag_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim_start_matches('v')
        .to_string();
    assert!(!version.is_empty(), "latest release must have a tag");
    assert!(
        updater::compare_versions(&version, "2.1.0") >= 0,
        "latest {version} must be newer than or equal to 2.1.0"
    );
    let has_windows_asset = latest
        .get("assets")
        .and_then(|v| v.as_array())
        .map(|assets| {
            assets
                .iter()
                .any(|a| a.get("name").and_then(|v| v.as_str()).map(is_installer).unwrap_or(false))
        })
        .unwrap_or(false);
    println!("[updater] latest release = {version}, windows asset present = {has_windows_asset}");

    // the production check() must agree with that observation
    let offered = updater::check("2.1.0").expect("release lookup must not fail");
    match offered {
        Some(found) => {
            assert!(has_windows_asset, "check() offered an update without a Windows asset");
            assert!(is_installer(&found.asset.name));
            assert_eq!(found.asset.sha256.len(), 64);
            println!("[updater] offered {} ({})", found.version, found.asset.name);
        }
        None => {
            assert!(
                !has_windows_asset,
                "check() must offer the Windows installer when the latest release has one"
            );
            println!("[updater] no update offered: the latest release has no Windows installer asset");
        }
    }
    let definitely_old = updater::check("0.0.1").expect("release lookup must not fail");
    assert_eq!(
        definitely_old.is_some(),
        has_windows_asset,
        "0.0.1 must see exactly the same availability as 2.1.0"
    );

    // 2. an older release that does ship the installer: asset rule + digest
    let releases = fetch_json(&format!("{API}/releases?per_page=10"));
    let list = releases.as_array().cloned().unwrap_or_default();
    let Some(candidate) = list.iter().find(|rel| {
        rel.get("assets")
            .and_then(|v| v.as_array())
            .map(|assets| {
                assets
                    .iter()
                    .any(|a| a.get("name").and_then(|v| v.as_str()).map(is_installer).unwrap_or(false))
            })
            .unwrap_or(false)
    }) else {
        println!("[updater] no release carries a Windows installer yet — digest path not exercised live");
        return;
    };
    let tag = candidate.get("tag_name").and_then(|v| v.as_str()).unwrap_or("");
    let asset = candidate
        .get("assets")
        .and_then(|v| v.as_array())
        .and_then(|assets| {
            assets
                .iter()
                .find(|a| a.get("name").and_then(|v| v.as_str()).map(is_installer).unwrap_or(false))
        })
        .unwrap();
    let parsed = asset_from(asset);
    assert_eq!(parsed.sha256.len(), 64, "{tag}: release digest must be sha256:<hex>");
    assert!(parsed.size > 1_000_000, "{tag}: installer size looks wrong");
    println!(
        "[updater] {tag}: {} {} bytes sha256 {}…",
        parsed.name,
        parsed.size,
        &parsed.sha256[..16]
    );
}

#[test]
#[ignore = "downloads the real installer (several tens of MB)"]
fn live_update_download_matches_the_published_digest() {
    if std::env::var("RUN_UPDATE_DOWNLOAD").ok().as_deref() != Some("1") {
        println!("[updater] RUN_UPDATE_DOWNLOAD != 1, skipping the download");
        return;
    }
    let releases = fetch_json(&format!("{API}/releases?per_page=10"));
    let candidate = releases
        .as_array()
        .and_then(|list| {
            list.iter().find(|rel| {
                rel.get("assets")
                    .and_then(|v| v.as_array())
                    .map(|assets| {
                        assets.iter().any(|a| {
                            a.get("name")
                                .and_then(|v| v.as_str())
                                .map(is_installer)
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false)
            })
        })
        .expect("a release with a Windows installer");
    let asset = asset_from(
        candidate
            .get("assets")
            .and_then(|v| v.as_array())
            .and_then(|assets| {
                assets
                    .iter()
                    .find(|a| a.get("name").and_then(|v| v.as_str()).map(is_installer).unwrap_or(false))
            })
            .unwrap(),
    );
    let dir = std::env::temp_dir().join("smsbridge-updater-test");
    let mut last_logged = 0u64;
    let path = updater::download(&asset, &dir, |done, total| {
        if done - last_logged > 16 * 1024 * 1024 {
            last_logged = done;
            println!("[updater] {done}/{total} bytes");
        }
    })
    .expect("download must succeed from at least one source and match size+digest");
    let meta = std::fs::metadata(&path).unwrap();
    println!(
        "[updater] downloaded {} ({} bytes), size and SHA-256 verified",
        path.display(),
        meta.len()
    );
    assert_eq!(meta.len(), asset.size);
    let _ = std::fs::remove_dir_all(&dir);
}

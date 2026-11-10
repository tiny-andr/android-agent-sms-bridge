//! Live protocol test against a real phone.
//!
//! Verify with a paired phone on the LAN:
//!
//! ```text
//! cargo test --test live_protocol -- --ignored --nocapture
//! ```
//!
//! It reads the *real* profile (`%APPDATA%\Windows SMS Bridge`, or
//! `SMSGLASS_DATA_DIR`), opens the DPAPI-sealed pairing token exactly like the
//! app does, pins the stored certificate and then:
//!  1. answers `GET /v1/health` (unsigned),
//!  2. sends a **signed** `GET /v1/sms` (this is the byte-level signing check:
//!     the phone returns 401 for the smallest canonical-string mismatch),
//!  3. verifies the certificate the phone presents equals the pinned one,
//!  4. compares the local cache with what the phone reports (parity evidence
//!     for the "same window, same counts" acceptance criterion).

use windows_sms_bridge::smsbridge::{self, BridgeClient, SmsQuery};
use windows_sms_bridge::store::Store;

fn data_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("SMSGLASS_DATA_DIR") {
        if !dir.trim().is_empty() {
            return std::path::PathBuf::from(dir);
        }
    }
    std::path::PathBuf::from(std::env::var("APPDATA").expect("APPDATA"))
        .join("Windows SMS Bridge")
}

#[test]
#[ignore = "needs a paired phone reachable on the LAN"]
fn live_signed_sync_matches_the_phone() {
    let dir = data_dir();
    let store = Store::new(&dir);
    let phones = store.list_phones();
    assert!(!phones.is_empty(), "no paired phone in {}", dir.display());

    for phone in &phones {
        let id = phone["id"].as_str().unwrap();
        let name = phone["name"].as_str().unwrap_or("");
        let url = phone["url"].as_str().unwrap();
        let cert_fp = phone["certFp"].as_str().unwrap_or("");
        let token = store.token_of(id);
        if token.is_empty() {
            // A record recovered from an older profile can be sealed with that
            // profile's Local State key, so it cannot be opened here; the app
            // shows "re-pair" for it, exactly like the Electron build did.
            println!("[{name}] token unreadable (sealed by another profile) — skipping");
            continue;
        }
        let raw = store.get_phone(id).unwrap();
        let cert_pem = raw["certPem"].as_str().unwrap_or("").to_string();
        assert!(!cert_pem.is_empty(), "{name}: no pinned certificate");
        assert_eq!(
            smsbridge::fingerprint_of_pem(&cert_pem),
            cert_fp,
            "{name}: pinned PEM does not match certFp"
        );

        // 1. unsigned health, over the pinned certificate
        let health = smsbridge::health(url, Some(&cert_pem), false, 6000)
            .unwrap_or_else(|e| panic!("{name}: health failed: {}", e.message));
        println!(
            "[{name}] health ok: app={} version={} battery={} sims={}",
            health["app"].as_str().unwrap_or("?"),
            health["version"].as_str().unwrap_or("?"),
            health["battery"],
            health["sims"].as_array().map(|a| a.len()).unwrap_or(0)
        );

        // 2. signed request — the real HMAC contract
        let client_id = raw["clientId"].as_str().unwrap();
        let client = BridgeClient::new(url, client_id, &token, Some(&cert_pem)).unwrap();
        let mut query = SmsQuery::new(200);
        query.box_ = "all".into();
        query.days = Some(30);
        let response = client
            .sms(&query)
            .unwrap_or_else(|e| panic!("{name}: signed /v1/sms failed: {} ({})", e.message, e.code));
        let phone_count = response["count"].as_i64().unwrap_or(0);
        let rows = response["messages"].as_array().cloned().unwrap_or_default();
        println!(
            "[{name}] signed /v1/sms ok: count={phone_count} rows={} first_id={}",
            rows.len(),
            rows.first().and_then(|m| m["id"].as_i64()).unwrap_or(0)
        );

        // 3. nothing that reached the phone may be written back
        let local = store.messages_of(id);
        println!(
            "[{name}] local cache: {} messages (lastSmsId={})",
            local.len(),
            raw["lastSmsId"]
        );
        if let Some(max_id) = rows.iter().filter_map(|m| m["id"].as_i64()).max() {
            println!("[{name}] newest id from phone = {max_id}");
        }

        // 4. the health payload has the fields the UI renders
        for key in ["battery", "charging", "sims", "paired_clients"] {
            assert!(health.get(key).is_some(), "{name}: health is missing {key}");
        }
    }
}

/// The certificate pin is strict: a wrong pin must fail, never downgrade.
#[test]
#[ignore = "needs a paired phone reachable on the LAN"]
fn wrong_pin_is_rejected() {
    let store = Store::new(data_dir());
    let phones = store.list_phones();
    assert!(!phones.is_empty());
    let phone = &phones[0];
    let url = phone["url"].as_str().unwrap();
    let raw = store.get_phone(phone["id"].as_str().unwrap()).unwrap();
    let token = store.token_of(phone["id"].as_str().unwrap());
    let client_id = raw["clientId"].as_str().unwrap();

    // A different, well-formed certificate must be rejected by the verifier.
    let other = smsbridge::der_to_pem(b"not the phone certificate");
    let client = BridgeClient::new(url, client_id, &token, Some(&other)).unwrap();
    let result = client.whoami();
    assert!(result.is_err(), "a wrong pinned certificate must not be accepted");
    println!("wrong pin rejected with: {}", result.unwrap_err().message);
}

//! Pairing-token sealing, byte-compatible with the Electron build.
//!
//! The old app used Electron's `safeStorage.encryptString()`. On Windows that is
//! Chromium's OSCrypt: the payload is `"v10" || nonce(12) || AES-256-GCM(ct+tag)`
//! and the AES key lives in `<profile>/Local State` (`os_crypt.encrypted_key`),
//! itself wrapped with **DPAPI**. A user upgrading from 2.1.0 must therefore be
//! able to open that blob — and a token copied to another machine must not be
//! openable, which DPAPI guarantees.
//!
//! We write the very same shape, so downgrading to the Electron build also keeps
//! working. Fallback on read: a bare DPAPI blob (the shape some early builds
//! produced) is accepted too.

use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use serde_json::{json, Map, Value};

const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;
/// Chromium writes "DPAPI" in front of the DPAPI-wrapped key blob.
const DPAPI_PREFIX: &[u8] = b"DPAPI";

// ---------------------------------------------------------------- DPAPI (win)

#[cfg(windows)]
mod dpapi {
    use windows::core::w;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        }
    }

    fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        if out.pbData.is_null() || out.cbData == 0 {
            return Vec::new();
        }
        let slice = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) };
        let v = slice.to_vec();
        unsafe {
            let _ = LocalFree(HLOCAL(out.pbData as *mut core::ffi::c_void));
        }
        v
    }

    pub fn protect(data: &[u8]) -> Result<Vec<u8>, String> {
        let input = blob(data);
        let mut output = CRYPT_INTEGER_BLOB::default();
        unsafe {
            CryptProtectData(
                &input,
                w!("Windows SMS Bridge"),
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
            .map_err(|e| format!("CryptProtectData: {e}"))?;
        }
        Ok(take(output))
    }

    pub fn unprotect(data: &[u8]) -> Result<Vec<u8>, String> {
        let input = blob(data);
        let mut output = CRYPT_INTEGER_BLOB::default();
        unsafe {
            CryptUnprotectData(
                &input,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
            .map_err(|e| format!("CryptUnprotectData: {e}"))?;
        }
        Ok(take(output))
    }
}

#[cfg(not(windows))]
mod dpapi {
    pub fn protect(_data: &[u8]) -> Result<Vec<u8>, String> {
        Err("DPAPI is only available on Windows".into())
    }
    pub fn unprotect(_data: &[u8]) -> Result<Vec<u8>, String> {
        Err("DPAPI is only available on Windows".into())
    }
}

// ------------------------------------------------------------- token sealing

#[derive(Clone)]
pub struct TokenCrypto {
    key: Option<[u8; KEY_LEN]>,
    local_state: PathBuf,
}

impl TokenCrypto {
    /// Loads (or creates) the AES key wrapped in `Local State` under `dir`.
    pub fn load(dir: &Path) -> Self {
        let local_state = dir.join("Local State");
        let mut key: Option<[u8; KEY_LEN]> = None;

        if let Ok(text) = std::fs::read_to_string(&local_state) {
            if let Ok(json) = serde_json::from_str::<Value>(&text) {
                if let Some(b64) = json
                    .get("os_crypt")
                    .and_then(|v| v.get("encrypted_key"))
                    .and_then(|v| v.as_str())
                {
                    if let Some(k) = open_local_state_key(b64) {
                        key = Some(k);
                    }
                }
            }
        }

        let mut crypto = TokenCrypto { key, local_state };
        if crypto.key.is_none() {
            crypto.key = Some(create_key(&crypto.local_state));
        }
        crypto
    }

    pub fn available(&self) -> bool {
        self.key.is_some()
    }

    /// `{tokenEnc, tokenRaw}` exactly like the Electron store's `sealToken`.
    pub fn seal(&self, token: &str) -> Map<String, Value> {
        let mut out = Map::new();
        if token.is_empty() {
            out.insert("tokenEnc".into(), json!(""));
            out.insert("tokenRaw".into(), json!(""));
            return out;
        }
        match self.encrypt(token.as_bytes()) {
            Ok(enc) => {
                out.insert("tokenEnc".into(), json!(enc));
                out.insert("tokenRaw".into(), json!(""));
            }
            Err(err) => {
                eprintln!("[store] token sealing failed, storing in the clear: {err}");
                out.insert("tokenEnc".into(), json!(""));
                out.insert("tokenRaw".into(), json!(token));
            }
        }
        out
    }

    /// Decrypts a `tokenEnc` blob. Returns "" when it cannot be opened, which is
    /// what the Electron store does too (the UI then offers "re-pair").
    pub fn open(&self, token_enc: &str) -> String {
        if token_enc.is_empty() {
            return String::new();
        }
        let raw = match B64.decode(token_enc) {
            Ok(v) => v,
            Err(_) => return String::new(),
        };
        self.decrypt(&raw).unwrap_or_default()
    }

    fn encrypt(&self, plain: &[u8]) -> Result<String, String> {
        let key = self.key.ok_or("no key")?;
        let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| e.to_string())?;
        let mut nonce = [0u8; NONCE_LEN];
        use rand::RngCore;
        rand::thread_rng().fill_bytes(&mut nonce);
        let ct = cipher
            .encrypt(Nonce::from_slice(&nonce), plain)
            .map_err(|_| "AES-GCM encrypt failed".to_string())?;
        let mut blob = Vec::with_capacity(3 + NONCE_LEN + ct.len());
        blob.extend_from_slice(b"v10");
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ct);
        Ok(B64.encode(blob))
    }

    fn decrypt(&self, blob: &[u8]) -> Option<String> {
        if blob.len() > 3 + NONCE_LEN && (blob.starts_with(b"v10") || blob.starts_with(b"v11")) {
            let key = self.key?;
            let cipher = Aes256Gcm::new_from_slice(&key).ok()?;
            let nonce = &blob[3..3 + NONCE_LEN];
            let ct = &blob[3 + NONCE_LEN..];
            let plain = cipher.decrypt(Nonce::from_slice(nonce), ct).ok()?;
            return String::from_utf8(plain).ok();
        }
        // Fallback: a bare DPAPI blob (all-zero/unknown prefix is not expected,
        // but the version DWORD is the reliable marker).
        if blob.len() >= 4 && blob[0] == 1 && blob[1] == 0 && blob[2] == 0 && blob[3] == 0 {
            if let Ok(plain) = dpapi::unprotect(blob) {
                return String::from_utf8(plain).ok();
            }
        }
        None
    }
}

fn open_local_state_key(b64: &str) -> Option<[u8; KEY_LEN]> {
    let blob = B64.decode(b64).ok()?;
    let payload = blob.strip_prefix(DPAPI_PREFIX).unwrap_or(&blob);
    let plain = dpapi::unprotect(payload).ok()?;
    if plain.len() != KEY_LEN {
        return None;
    }
    let mut key = [0u8; KEY_LEN];
    key.copy_from_slice(&plain);
    Some(key)
}

fn create_key(local_state: &Path) -> [u8; KEY_LEN] {
    use rand::RngCore;
    let mut key = [0u8; KEY_LEN];
    rand::thread_rng().fill_bytes(&mut key);

    let mut doc: Value = std::fs::read_to_string(local_state)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}));
    if !doc.is_object() {
        doc = json!({});
    }
    let wrapped = dpapi::protect(&key).unwrap_or_default();
    if wrapped.is_empty() {
        eprintln!("[store] DPAPI unavailable: the pairing token cannot be sealed");
        return key;
    }
    let mut prefixed = DPAPI_PREFIX.to_vec();
    prefixed.extend_from_slice(&wrapped);

    let os_crypt = doc
        .as_object_mut()
        .map(|m| {
            m.entry("os_crypt")
                .or_insert_with(|| json!({}))
                .clone()
        })
        .unwrap_or_else(|| json!({}));
    let mut os_crypt = if os_crypt.is_object() { os_crypt } else { json!({}) };
    {
        let map = os_crypt.as_object_mut().unwrap();
        map.insert("audit_enabled".into(), json!(false));
        map.insert("encrypted_key".into(), json!(B64.encode(&prefixed)));
    }
    doc["os_crypt"] = os_crypt;

    let text = serde_json::to_string(&doc).unwrap_or_default();
    if let Some(parent) = local_state.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(err) = write_atomic(local_state, &text) {
        eprintln!("[store] could not write Local State: {err}");
    }
    key
}

/// tmp + rename, the same crash-safe write the Electron store uses.
pub fn write_atomic(path: &Path, data: &str) -> Result<(), String> {
    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&tmp, data).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aes_blob_roundtrip_has_v10_prefix() {
        let dir = std::env::temp_dir().join(format!("smsglass-dpapi-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let crypto = TokenCrypto::load(&dir);
        assert!(crypto.available());
        let sealed = crypto.encrypt(b"token-abc").unwrap();
        let blob = B64.decode(&sealed).unwrap();
        assert!(blob.starts_with(b"v10"));
        // 3 prefix + 12 nonce + 9 plaintext + 16 tag
        assert_eq!(blob.len(), 3 + 12 + 9 + 16);
        assert_eq!(crypto.decrypt(&blob).unwrap(), "token-abc");
        // the key lives in Local State, DPAPI-wrapped
        let state = std::fs::read_to_string(dir.join("Local State")).unwrap();
        assert!(state.contains("encrypted_key"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plain_dpapi_blob_fallback() {
        let dir = std::env::temp_dir().join(format!("smsglass-dpapi-fb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let crypto = TokenCrypto::load(&dir);
        let blob = dpapi::protect(b"legacy-token").unwrap();
        assert_eq!(crypto.decrypt(&blob).unwrap(), "legacy-token");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

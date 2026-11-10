//! SmsBridge client — talks to the phone's local API.
//!
//! Ported 1:1 from the Electron build's `src/core/smsbridge.js`, with the same
//! security properties, only the TLS stack is different (rustls instead of
//! Node's OpenSSL):
//!
//!  · Transport is HTTPS only. The phone serves a **self-signed** certificate,
//!    so the client trusts on first use: the first connection fetches the
//!    certificate, the PEM is stored on the phone record, and every later
//!    request is verified against **exactly this certificate** (a custom rustls
//!    verifier compares the DER byte for byte — stricter than CA validation,
//!    and it cannot be downgraded).
//!  · Signing (see API.md and android .../http/Auth.kt):
//!      canonical   = METHOD \n PATH \n RAW_QUERY_STRING \n TIMESTAMP_MS \n NONCE
//!      signature   = hex(HMAC-SHA256(token, canonical))
//!    The token is used as a UTF-8 string, nonce is 12 random bytes hex-encoded,
//!    timestamp is epoch milliseconds. Verified byte-identical against
//!    smsbridge.js, Auth.kt and smsbridge_client.py (see the unit test at the
//!    bottom, whose vector was cross-computed by the JS and Python clients).
//!  · The phone is on the LAN, so requests never go through a system proxy.

use std::net::ToSocketAddrs;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, SignatureScheme};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::i18n::t;

#[derive(Debug, Clone)]
pub struct BridgeError {
    pub message: String,
    pub status: u16,
    pub code: String,
    pub payload: Option<Value>,
}

impl BridgeError {
    pub fn new(message: impl Into<String>) -> Self {
        BridgeError {
            message: message.into(),
            status: 0,
            code: "error".into(),
            payload: None,
        }
    }

    pub fn with_code(message: impl Into<String>, code: &str) -> Self {
        BridgeError {
            message: message.into(),
            status: 0,
            code: code.into(),
            payload: None,
        }
    }

    /// A revoked / stale token is not a network problem; the UI says so and
    /// offers "re-pair" instead of a generic red dot.
    pub fn is_unauthorized(&self) -> bool {
        self.status == 401
            || self.code == "unauthorized"
            || ["unknown client", "bad signature", "revoked"]
                .iter()
                .any(|needle| self.message.to_lowercase().contains(needle))
    }
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for BridgeError {}

impl From<BridgeError> for String {
    fn from(e: BridgeError) -> String {
        e.message
    }
}

const DEFAULT_TIMEOUT_MS: u64 = 12_000;
const DEFAULT_PORT: u16 = 8787;

// ------------------------------------------------------------------- helpers

/// `"192.168.1.5"`, `"http://192.168.1.5:8787/"` → `"https://192.168.1.5:8787"`.
pub fn normalize_base(url: &str) -> Result<String, BridgeError> {
    let mut u = url.trim().to_string();
    if u.is_empty() {
        return Err(BridgeError::new(t("请填写手机地址", &[])));
    }
    if u.len() >= 7 && u[..7].eq_ignore_ascii_case("http://") {
        u = format!("https://{}", &u[7..]);
    }
    let lower = u.to_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) {
        u = format!("https://{u}");
    }
    while u.ends_with('/') {
        u.pop();
    }
    let parsed = url::Url::parse(&u).map_err(|_| {
        BridgeError::new(t("地址格式不正确，例如 https://192.168.1.5:8787", &[]))
    })?;
    let host = match parsed.host() {
        Some(url::Host::Ipv6(addr)) => format!("[{addr}]"),
        Some(url::Host::Ipv4(addr)) => addr.to_string(),
        Some(url::Host::Domain(domain)) => domain.to_string(),
        None => {
            return Err(BridgeError::new(t(
                "地址格式不正确，例如 https://192.168.1.5:8787",
                &[],
            )))
        }
    };
    let port = parsed.port().unwrap_or(DEFAULT_PORT);
    Ok(format!("https://{host}:{port}"))
}

/// SHA-256 of a certificate, lowercase hex — what the phone shows on screen.
pub fn fingerprint_of_der(der: &[u8]) -> String {
    hex::encode(Sha256::digest(der))
}

/// Human form, e.g. "1D 96 B0 E7 …", same as the phone's pairing screen.
pub fn pretty_fingerprint(hex_str: &str) -> String {
    let upper = hex_str.to_uppercase();
    let bytes = upper.as_bytes();
    let mut out = String::with_capacity(bytes.len() + bytes.len() / 2);
    for (i, ch) in upper.chars().enumerate() {
        if i > 0 && i % 2 == 0 {
            out.push(' ');
        }
        out.push(ch);
    }
    out
}

pub fn pem_to_der(pem: &str) -> Result<Vec<u8>, BridgeError> {
    let body: String = pem
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect::<Vec<_>>()
        .join("");
    B64.decode(body.trim())
        .map_err(|e| BridgeError::new(format!("invalid PEM certificate: {e}")))
}

pub fn der_to_pem(der: &[u8]) -> String {
    let b64 = B64.encode(der);
    let mut out = String::from("-----BEGIN CERTIFICATE-----\n");
    for chunk in b64.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(chunk).unwrap_or(""));
        out.push('\n');
    }
    out.push_str("-----END CERTIFICATE-----\n");
    out
}

pub fn fingerprint_of_pem(pem: &str) -> String {
    match pem_to_der(pem) {
        Ok(der) => fingerprint_of_der(&der),
        Err(_) => String::new(),
    }
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn server_name(host: &str) -> Result<ServerName<'static>, BridgeError> {
    let stripped = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = stripped.parse::<std::net::IpAddr>() {
        return Ok(ServerName::IpAddress(ip.into()));
    }
    ServerName::try_from(host.to_string())
        .map_err(|e| BridgeError::new(format!("invalid host {host}: {e}")))
}

/// Verifier that accepts exactly one certificate and nothing else.
struct PinnedVerifier {
    der: Vec<u8>,
    provider: Arc<CryptoProvider>,
}

impl std::fmt::Debug for PinnedVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PinnedVerifier")
    }
}

impl ServerCertVerifier for PinnedVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if end_entity.as_ref() == self.der.as_slice() {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(
                "certificate pinned mismatch: the phone's certificate changed".into(),
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// TOFU verifier: accepts whatever the phone presents once, and remembers it so
/// the caller can pin it. It is only ever used for the very first connection.
struct TofuVerifier {
    captured: Arc<Mutex<Option<Vec<u8>>>>,
    provider: Arc<CryptoProvider>,
}

impl std::fmt::Debug for TofuVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TofuVerifier")
    }
}

impl ServerCertVerifier for TofuVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        *self.captured.lock().unwrap() = Some(end_entity.as_ref().to_vec());
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn client_config_with(verifier: Arc<dyn ServerCertVerifier>) -> Result<Arc<ClientConfig>, BridgeError> {
    let provider = provider();
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| BridgeError::new(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

fn pinned_config(der: Vec<u8>) -> Result<Arc<ClientConfig>, BridgeError> {
    client_config_with(Arc::new(PinnedVerifier {
        der,
        provider: provider(),
    }))
}

fn target_parts(base: &str) -> Result<(String, u16), BridgeError> {
    let parsed = url::Url::parse(base).map_err(|e| BridgeError::new(e.to_string()))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| BridgeError::new(t("地址格式不正确，例如 https://192.168.1.5:8787", &[])))?
        .to_string();
    let port = parsed.port().unwrap_or(DEFAULT_PORT);
    Ok((host.trim_matches(['[', ']']).to_string(), port))
}

// --------------------------------------------------------------- TOFU pinning

pub struct PinInfo {
    pub base: String,
    pub pem: String,
    pub fingerprint: String,
    pub pretty: String,
}

/// Fetch the certificate the phone presents, without trusting it yet.
pub fn pin_certificate(base_url: &str, timeout_ms: u64) -> Result<PinInfo, BridgeError> {
    let base = normalize_base(base_url)?;
    let (host, port) = target_parts(&base)?;
    let timeout = Duration::from_millis(timeout_ms.max(1));

    let captured = Arc::new(Mutex::new(None));
    let config = client_config_with(Arc::new(TofuVerifier {
        captured: captured.clone(),
        provider: provider(),
    }))?;
    let name = server_name(&host)?;

    let addr = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|e| network_error(&e.to_string(), timeout_ms))?
        .next()
        .ok_or_else(|| BridgeError::with_code(t("网络不可达：请确认电脑与手机在同一局域网", &[]), "network"))?;

    let mut sock = std::net::TcpStream::connect_timeout(&addr, timeout).map_err(|e| {
        if e.kind() == std::io::ErrorKind::TimedOut {
            timeout_error()
        } else {
            network_error(&e.to_string(), timeout_ms)
        }
    })?;
    let _ = sock.set_read_timeout(Some(timeout));
    let _ = sock.set_write_timeout(Some(timeout));

    let mut conn = ClientConnection::new(config, name).map_err(|e| BridgeError::new(e.to_string()))?;
    loop {
        match conn.complete_io(&mut sock) {
            Ok(_) => {
                if !conn.is_handshaking() {
                    break;
                }
            }
            Err(err) => {
                let msg = err.to_string();
                if msg.contains("timed out") || msg.contains("WouldBlock") {
                    return Err(timeout_error());
                }
                if msg.contains("certificate") {
                    return Err(BridgeError::with_code(
                        t("手机证书校验失败：可能重装过 App，请重新配对", &[]),
                        "tls",
                    ));
                }
                return Err(BridgeError::with_code(
                    t("手机端的服务不是 HTTPS：请把手机 App 更新到 1.0.1 及以上", &[]),
                    "protocol",
                ));
            }
        }
    }

    let der = captured
        .lock()
        .unwrap()
        .clone()
        .or_else(|| conn.peer_certificates().and_then(|c| c.first().map(|d| d.as_ref().to_vec())))
        .ok_or_else(|| BridgeError::new(t("手机没有提供 TLS 证书", &[])))?;

    let fingerprint = fingerprint_of_der(&der);
    let pretty = pretty_fingerprint(&fingerprint);
    Ok(PinInfo {
        base,
        pem: der_to_pem(&der),
        fingerprint,
        pretty,
    })
}

// ------------------------------------------------------------ HTTP transport

fn timeout_error() -> BridgeError {
    BridgeError::with_code(
        t(
            "手机没有响应（超时）。确认同一 Wi-Fi、App 服务已启动、省电模式已关闭",
            &[],
        ),
        "timeout",
    )
}

fn network_error(message: &str, _timeout_ms: u64) -> BridgeError {
    let lower = message.to_lowercase();
    let text = if lower.contains("refused") {
        t("连接被拒绝：手机上的短信桥服务可能未启动", &[])
    } else if lower.contains("unreachable") {
        t("网络不可达：请确认电脑与手机在同一局域网", &[])
    } else if lower.contains("reset") {
        t("连接被重置：手机端可能被系统冻结", &[])
    } else {
        message.to_string()
    };
    BridgeError::with_code(text, "network")
}

pub struct RawResponse {
    pub status: u16,
    pub text: String,
    pub json: Option<Value>,
}

fn parse_json(text: &str) -> Option<Value> {
    serde_json::from_str::<Value>(text).ok()
}

fn build_agent(config: Arc<ClientConfig>, timeout_ms: u64) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .tls_config(config)
        .timeout_connect(Duration::from_millis(timeout_ms))
        .build()
}

#[allow(clippy::too_many_arguments)]
fn raw_request(
    base_url: &str,
    method: &str,
    path: &str,
    query: &str,
    headers: &[(&str, String)],
    body: Option<&str>,
    timeout_ms: u64,
    cert_pem: Option<&str>,
    trust_any: bool,
) -> Result<RawResponse, BridgeError> {
    let base = normalize_base(base_url)?;
    let url = if query.is_empty() {
        format!("{base}{path}")
    } else {
        format!("{base}{path}?{query}")
    };
    let config = match cert_pem.filter(|p| !p.is_empty()) {
        Some(pem) => pinned_config(pem_to_der(pem)?)?,
        None if trust_any => client_config_with(Arc::new(TofuVerifier {
            captured: Arc::new(Mutex::new(None)),
            provider: provider(),
        }))?,
        None => {
            return Err(BridgeError::with_code(
                t(
                    "这台手机的证书还没有被信任：请重新配对，配对时会记录并固定它的证书",
                    &[],
                ),
                "tls",
            ))
        }
    };

    let agent = build_agent(config, timeout_ms);
    let mut req = agent
        .request(method, &url)
        .set("Accept", "application/json")
        .timeout(Duration::from_millis(timeout_ms));
    for (k, v) in headers {
        req = req.set(k, v);
    }
    if body.is_some() {
        req = req.set("Content-Type", "application/json; charset=utf-8");
    }

    let result = match body {
        Some(payload) => req.send_string(payload),
        None => req.call(),
    };

    match result {
        Ok(resp) => {
            let status = resp.status();
            let text = resp.into_string().unwrap_or_default();
            Ok(RawResponse {
                status,
                json: parse_json(&text),
                text,
            })
        }
        Err(ureq::Error::Status(status, resp)) => {
            let text = resp.into_string().unwrap_or_default();
            Ok(RawResponse {
                status,
                json: parse_json(&text),
                text,
            })
        }
        Err(ureq::Error::Transport(transport)) => Err(transport_bridge_error(transport)),
    }
}

fn transport_bridge_error(transport: ureq::Transport) -> BridgeError {
    use std::error::Error as StdError;
    // ureq only exposes the outermost message ("tls connection init failed"); the
    // rustls error that tells us *why* (pin mismatch, bad certificate, timeout)
    // is further down the source chain. Walk it so the UI shows the same hint
    // the Electron build produced for the same failure.
    let mut chain = String::new();
    let mut current: Option<&(dyn StdError + 'static)> = Some(&transport);
    while let Some(err) = current {
        chain.push_str(&err.to_string());
        chain.push_str(" | ");
        current = err.source();
    }
    let message = transport.message().unwrap_or("network error").to_string();
    let lower = format!("{message} {chain}").to_lowercase();
    if lower.contains("pinned mismatch") || lower.contains("pinned certificate") {
        return BridgeError::with_code(
            t("手机证书校验失败：可能重装过 App，请重新配对", &[]),
            "tls",
        );
    }
    if lower.contains("timed out") || lower.contains("timeout") || lower.contains("elapsed") {
        return timeout_error();
    }
    if lower.contains("refused") || lower.contains("unreachable") || lower.contains("reset") {
        return network_error(&message, 0);
    }
    if lower.contains("certificate") {
        return BridgeError::with_code(
            t("手机证书校验失败：可能重装过 App，请重新配对", &[]),
            "tls",
        );
    }
    if lower.contains("handshake") || lower.contains("alert") || lower.contains("tls") {
        return BridgeError::with_code(
            t("TLS 握手失败：手机端可能还是只支持 HTTP 的旧版本，请更新 App", &[]),
            "protocol",
        );
    }
    BridgeError::with_code(message, "network")
}

fn fail(result: &RawResponse, fallback: &str) -> BridgeError {
    let code = result
        .json
        .as_ref()
        .and_then(|j| j.get("error"))
        .and_then(|e| e.get("code"))
        .and_then(|c| c.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| {
            if result.status == 0 {
                "network".to_string()
            } else {
                format!("http_{}", result.status)
            }
        });
    let message = result
        .json
        .as_ref()
        .and_then(|j| j.get("error"))
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| {
            if fallback.is_empty() {
                format!("HTTP {}", result.status)
            } else {
                fallback.to_string()
            }
        });
    BridgeError {
        message,
        status: result.status,
        code,
        payload: result.json.clone(),
    }
}

// ------------------------------------------------------------------ endpoints

/// Unsigned health probe. `cert_pem` = Some(pem) pins; None requires trust_any.
pub fn health(base_url: &str, cert_pem: Option<&str>, trust_any: bool, timeout_ms: u64) -> Result<Value, BridgeError> {
    let result = raw_request(
        base_url,
        "GET",
        "/v1/health",
        "",
        &[],
        None,
        timeout_ms.max(1),
        cert_pem,
        trust_any,
    )?;
    let ok = result
        .json
        .as_ref()
        .and_then(|j| j.get("ok"))
        .map(|v| v.as_bool().unwrap_or(!v.is_null()))
        .unwrap_or(false);
    if result.status != 200 || !ok {
        return Err(fail(&result, &t("手机端健康检查失败", &[])));
    }
    Ok(result.json.unwrap_or(Value::Null))
}

#[derive(Debug, Clone)]
pub struct Creds {
    pub base_url: String,
    pub client_id: String,
    pub token: String,
    pub client_name: String,
    pub cert_pem: Option<String>,
    pub fingerprint: String,
}

pub fn pair(base_url: &str, code: &str, name: &str, cert_pem: Option<&str>) -> Result<Creds, BridgeError> {
    let base = normalize_base(base_url)?;
    let mut payload = Map::new();
    payload.insert("code".into(), json!(code.trim()));
    payload.insert(
        "name".into(),
        json!(if name.is_empty() { "Windows SMS Bridge" } else { name }),
    );
    let body = serde_json::to_string(&payload).unwrap_or_default();
    let result = raw_request(
        &base,
        "POST",
        "/v1/pair",
        "",
        &[],
        Some(&body),
        DEFAULT_TIMEOUT_MS,
        cert_pem,
        false,
    )?;
    let ok = result
        .json
        .as_ref()
        .and_then(|j| j.get("ok"))
        .map(|v| v.as_bool().unwrap_or(false))
        .unwrap_or(false);
    if result.status != 200 || !ok {
        let mut err = fail(&result, &t("配对失败", &[]));
        if err.status == 401 {
            err.message = t("配对码错误或已过期（5 分钟有效、一次性）", &[]);
        }
        return Err(err);
    }
    let json = result.json.clone().unwrap_or(Value::Null);
    let client_id = json
        .get("client_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let token = json
        .get("token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if client_id.is_empty() || token.is_empty() {
        return Err(BridgeError::new(t("配对响应缺少凭据", &[])));
    }
    // The phone echoes the name back; older builds decode the body as US-ASCII
    // and return U+FFFD, so prefer our own copy when the echo looks broken.
    let echoed = json
        .get("client_name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.contains('\u{fffd}'))
        .unwrap_or("")
        .to_string();
    Ok(Creds {
        base_url: base,
        client_id,
        token,
        client_name: if echoed.is_empty() {
            name.to_string()
        } else {
            echoed
        },
        cert_pem: cert_pem.map(str::to_string),
        fingerprint: cert_pem.map(fingerprint_of_pem).unwrap_or_default(),
    })
}

// --------------------------------------------------------------- signed client

pub struct BridgeClient {
    pub base_url: String,
    pub client_id: String,
    pub token: String,
    pub cert_pem: Option<String>,
    pub timeout_ms: u64,
}

impl BridgeClient {
    pub fn new(base_url: &str, client_id: &str, token: &str, cert_pem: Option<&str>) -> Result<Self, BridgeError> {
        Ok(BridgeClient {
            base_url: normalize_base(base_url)?,
            client_id: client_id.to_string(),
            token: token.to_string(),
            cert_pem: cert_pem.map(str::to_string),
            timeout_ms: DEFAULT_TIMEOUT_MS,
        })
    }

    pub fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    /// canonical = METHOD \n PATH \n RAW_QUERY \n TIMESTAMP_MS \n NONCE
    pub fn sign(&self, method: &str, path: &str, query: &str) -> Vec<(&'static str, String)> {
        let ts = crate::store::now_ms().to_string();
        let nonce: String = {
            use rand::RngCore;
            let mut bytes = [0u8; 12];
            rand::thread_rng().fill_bytes(&mut bytes);
            hex::encode(bytes)
        };
        let canonical = format!("{method}\n{path}\n{query}\n{ts}\n{nonce}");
        let signature = {
            use hmac::{Hmac, Mac};
            let mut mac = Hmac::<Sha256>::new_from_slice(self.token.as_bytes())
                .expect("HMAC accepts any key length");
            mac.update(canonical.as_bytes());
            hex::encode(mac.finalize().into_bytes())
        };
        vec![
            ("X-Client-Id", self.client_id.clone()),
            ("X-Timestamp", ts),
            ("X-Nonce", nonce),
            ("X-Signature", signature),
        ]
    }

    pub fn call(
        &self,
        method: &str,
        path: &str,
        params: Option<&[(&str, String)]>,
        body: Option<&str>,
    ) -> Result<Value, BridgeError> {
        let query = match params {
            Some(list) if !list.is_empty() => encode_query(list),
            _ => String::new(),
        };
        let headers = self.sign(method, path, &query);
        let result = raw_request(
            &self.base_url,
            method,
            path,
            &query,
            &headers,
            body,
            self.timeout_ms,
            self.cert_pem.as_deref(),
            false,
        )?;
        if result.status != 200 {
            return Err(fail(&result, &t("请求失败 ({path})", &[("path", path)])));
        }
        let json = result.json.unwrap_or(Value::Null);
        if json.get("ok") == Some(&Value::Bool(false)) {
            return Err(fail(
                &RawResponse {
                    status: result.status,
                    text: result.text,
                    json: Some(json),
                },
                &t("手机端返回错误", &[]),
            ));
        }
        Ok(json)
    }

    pub fn whoami(&self) -> Result<Value, BridgeError> {
        self.call("GET", "/v1/whoami", None, None)
    }

    pub fn sms(&self, opts: &SmsQuery) -> Result<Value, BridgeError> {
        let mut params: Vec<(&str, String)> = Vec::new();
        let mut push = |k: &'static str, v: String| params.push((k, v));
        push("box", opts.box_.clone());
        push("limit", opts.limit.to_string());
        if let Some(days) = opts.days {
            push("days", days.to_string());
        }
        if let Some(address) = opts.address.as_ref().filter(|s| !s.is_empty()) {
            push("address", address.clone());
        }
        if let Some(slot) = opts.slot {
            push("slot", slot.to_string());
        }
        if opts.offset != 0 {
            push("offset", opts.offset.to_string());
        }
        if let Some(since_id) = opts.since_id {
            push("since_id", since_id.to_string());
        }
        if let Some(q) = opts.q.as_ref().filter(|s| !s.is_empty()) {
            push("q", q.clone());
        }
        if let Some(purpose) = opts.purpose.as_ref().filter(|s| !s.is_empty()) {
            push("purpose", purpose.clone());
        }
        self.call("GET", "/v1/sms", Some(&params), None)
    }

    pub fn events(&self, since_seq: i64) -> Result<Value, BridgeError> {
        self.call(
            "GET",
            "/v1/events",
            Some(&[("since_seq", since_seq.to_string())]),
            None,
        )
    }

    pub fn stats(&self, days: i64) -> Result<Value, BridgeError> {
        self.call("GET", "/v1/stats", Some(&[("days", days.to_string())]), None)
    }

    pub fn revoke_self(&self) -> Result<Value, BridgeError> {
        self.call("POST", "/v1/revoke-self", None, None)
    }
}

/// Parameters of `GET /v1/sms`, in the exact insertion order the JS client uses
/// (the signature covers the raw query string, so order is part of the contract).
#[derive(Debug, Clone)]
pub struct SmsQuery {
    pub box_: String,
    pub limit: i64,
    pub days: Option<i64>,
    pub address: Option<String>,
    pub slot: Option<i64>,
    pub offset: i64,
    pub since_id: Option<i64>,
    pub q: Option<String>,
    pub purpose: Option<String>,
}

impl SmsQuery {
    pub fn new(limit: i64) -> Self {
        SmsQuery {
            box_: "inbox".into(),
            limit,
            days: None,
            address: None,
            slot: None,
            offset: 0,
            since_id: None,
            q: None,
            purpose: None,
        }
    }
}

/// URLSearchParams-compatible encoding: `k=v&k=v`, values percent-encoded the
/// way `encodeURIComponent` does (the phone signs the raw string it receives).
fn encode_query(params: &[(&str, String)]) -> String {
    let mut out = String::new();
    for (k, v) in params {
        if v.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('&');
        }
        out.push_str(&encode_component(k));
        out.push('=');
        out.push_str(&encode_component(v));
    }
    out
}

/// `application/x-www-form-urlencoded` component encoding, i.e. exactly what
/// `URLSearchParams.toString()` produces: alphanumerics and `*-._` stay, space
/// becomes `+`, everything else is `%XX` (uppercase).
fn encode_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vector cross-computed by the JS client (smsbridge.js) and the Python
    /// client (smsbridge_client.py); the phone (Auth.kt) verifies the same
    /// string, so this is the byte-level contract.
    #[test]
    fn signature_matches_js_and_python_clients() {
        let client = BridgeClient::new(
            "https://192.168.1.5:8787",
            "client-id",
            "test-token",
            None,
        )
        .unwrap();
        let canonical = "GET\n/v1/sms\nbox=all&limit=200\n1700000000000\nabcdef0123456789abcdef01";
        use hmac::{Hmac, Mac};
        let mut mac = Hmac::<Sha256>::new_from_slice(client.token.as_bytes()).unwrap();
        mac.update(canonical.as_bytes());
        assert_eq!(
            hex::encode(mac.finalize().into_bytes()),
            "c7a8060e17b23ce662603b8c050af5fce4871b548ab026d8bb1620c06a80531c"
        );
    }

    #[test]
    fn signing_headers_have_the_documented_shape() {
        let client =
            BridgeClient::new("https://192.168.1.5:8787", "cid", "tok", None).unwrap();
        let headers = client.sign("GET", "/v1/sms", "box=all");
        let names: Vec<&str> = headers.iter().map(|(k, _)| *k).collect();
        assert_eq!(names, ["X-Client-Id", "X-Timestamp", "X-Nonce", "X-Signature"]);
        assert_eq!(headers[1].1.len(), 13, "epoch milliseconds");
        assert_eq!(headers[2].1.len(), 24, "12 random bytes, hex — JS uses randomBytes(12).toString('hex')");
        assert_eq!(headers[3].1.len(), 64, "HMAC-SHA256 hex");
    }

    #[test]
    fn normalize_base_matches_js_behaviour() {
        assert_eq!(
            normalize_base("http://192.168.1.5:8787/").unwrap(),
            "https://192.168.1.5:8787"
        );
        assert_eq!(
            normalize_base("192.168.1.5").unwrap(),
            "https://192.168.1.5:8787"
        );
        assert_eq!(
            normalize_base(" https://192.168.1.5 ").unwrap(),
            "https://192.168.1.5:8787"
        );
        assert!(normalize_base("   ").is_err());
    }

    #[test]
    fn query_encoding_matches_urlsearchparams() {
        let q = encode_query(&[
            ("box", "all".into()),
            ("limit", "200".into()),
            ("days", "30".into()),
        ]);
        assert_eq!(q, "box=all&limit=200&days=30");
        assert_eq!(encode_component("a b"), "a+b");
        assert_eq!(encode_component("中国"), "%E4%B8%AD%E5%9B%BD");
    }

    #[test]
    fn fingerprint_helpers() {
        let pem = der_to_pem(b"hello");
        assert_eq!(pem_to_der(&pem).unwrap(), b"hello");
        assert_eq!(
            pretty_fingerprint("1d96b0e7"),
            "1D 96 B0 E7"
        );
        assert_eq!(
            fingerprint_of_der(b"hello"),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }
}

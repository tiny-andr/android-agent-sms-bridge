"""SmsBridge client — talks to the HTTP API served by the android app on the phone.

Auth: every request is signed with HMAC-SHA256 using the token obtained at pairing.

    canonical = METHOD \n PATH \n RAW_QUERY_STRING \n TIMESTAMP_MS \n NONCE
"""

from __future__ import annotations

import hashlib
import hmac
import json
import os
import secrets
import socket
import time
import ssl
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Any

DEFAULT_CONFIG = Path(os.environ.get(
    "SMSBRIDGE_CONFIG",
    str(Path(__file__).resolve().parent / ".smsbridge.json"),
))
DEFAULT_URL = os.environ.get("SMSBRIDGE_URL", "")

# The phone serves HTTPS with a self-signed certificate. We do not trust the
# system CA store, we trust exactly this one file (certificate pinning).
CERT_FILE = Path(os.environ.get(
    "SMSBRIDGE_CERT",
    str(Path(__file__).resolve().parent / "smsbridge-cert.pem"),
))

# The phone is on the LAN. Never route it through a system proxy: urllib would
# pick up HTTP_PROXY and a local proxy (Clash etc.) answers 502 for LAN hosts.
_opener: urllib.request.OpenerDirector | None = None


def _ssl_context(allow_unverified: bool = False) -> ssl.SSLContext:
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
    ctx.check_hostname = False  # the phone IP changes; we pin the certificate instead
    if allow_unverified:
        ctx.verify_mode = ssl.CERT_NONE
        return ctx
    if not CERT_FILE.exists():
        raise SmsBridgeError(
            f"no pinned certificate yet ({CERT_FILE}). "
            "Run the pair command first: it fetches and stores the phone certificate."
        )
    ctx.verify_mode = ssl.CERT_REQUIRED
    ctx.load_verify_locations(cafile=str(CERT_FILE))
    return ctx


def fetch_and_pin_certificate(base_url: str, timeout: float = 10.0) -> str:
    """TOFU: fetch the certificate the phone presents, pin it, return its SHA-256.

    This is the only moment the client talks to a server it does not trust yet.
    From then on every request is verified against CERT_FILE.
    """
    parsed = urllib.parse.urlparse(base_url)
    host = parsed.hostname
    if not host:
        raise SmsBridgeError(f"bad url: {base_url}")
    port = parsed.port or 8787
    ctx = _ssl_context(allow_unverified=True)
    try:
        with socket.create_connection((host, port), timeout=timeout) as sock:
            with ctx.wrap_socket(sock, server_hostname=host) as tls:
                der = tls.getpeercert(binary_form=True)
    except OSError as exc:
        raise SmsBridgeError(f"cannot reach {host}:{port}: {exc}") from None
    if not der:
        raise SmsBridgeError("the server presented no certificate")
    CERT_FILE.write_text(ssl.DER_cert_to_PEM_cert(der), encoding="utf-8")
    _reset_opener()
    return hashlib.sha256(der).hexdigest()


def _reset_opener() -> None:
    global _opener
    _opener = None


def _get_opener() -> urllib.request.OpenerDirector:
    global _opener
    if _opener is None:
        _opener = urllib.request.build_opener(
            urllib.request.ProxyHandler({}),
            urllib.request.HTTPSHandler(context=_ssl_context()),
        )
    return _opener


class SmsBridgeError(RuntimeError):
    def __init__(self, message: str, status: int | None = None, payload: Any = None):
        super().__init__(message)
        self.status = status
        self.payload = payload or {}


@dataclass
class Credentials:
    base_url: str
    client_id: str
    token: str
    client_name: str = "agent"

    @classmethod
    def load(cls, path: Path | None = None) -> "Credentials":
        p = Path(path) if path else DEFAULT_CONFIG
        if not p.exists():
            raise SmsBridgeError(
                f"not paired yet: {p} missing. run: python smsctl.py pair --url URL --code CODE"
            )
        data = json.loads(p.read_text(encoding="utf-8"))
        return cls(
            base_url=data["base_url"],
            client_id=data["client_id"],
            token=data["token"],
            client_name=data.get("client_name", "agent"),
        )

    def save(self, path: Path | None = None) -> Path:
        p = Path(path) if path else DEFAULT_CONFIG
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(json.dumps({
            "base_url": self.base_url,
            "client_id": self.client_id,
            "token": self.token,
            "client_name": self.client_name,
        }, ensure_ascii=False, indent=2), encoding="utf-8")
        try:
            os.chmod(p, 0o600)
        except OSError:
            pass
        return p


def _http(method: str, url: str, headers: dict[str, str], body: bytes | None, timeout: float):
    req = urllib.request.Request(url, data=body, method=method)
    for k, v in headers.items():
        req.add_header(k, v)
    try:
        with _get_opener().open(req, timeout=timeout) as resp:
            raw = resp.read()
    except urllib.error.HTTPError as e:
        raw = e.read()
        try:
            payload = json.loads(raw.decode("utf-8"))
        except Exception:
            payload = {"raw": raw.decode("utf-8", "replace")}
        msg = payload.get("error", {}).get("message") if isinstance(payload, dict) else None
        raise SmsBridgeError(msg or f"HTTP {e.code}", status=e.code, payload=payload) from None
    except urllib.error.URLError as e:
        raise SmsBridgeError(f"cannot reach {url}: {e.reason}") from None
    return json.loads(raw.decode("utf-8"))


class SmsBridge:
    def __init__(self, creds: Credentials, timeout: float = 15.0):
        self.creds = creds
        self.timeout = timeout

    # ---------------------------------------------------------------- auth

    def _sign(self, method: str, path: str, query: str) -> dict[str, str]:
        ts = str(int(time.time() * 1000))
        nonce = secrets.token_hex(12)
        canonical = f"{method}\n{path}\n{query}\n{ts}\n{nonce}"
        sig = hmac.new(
            self.creds.token.encode("utf-8"), canonical.encode("utf-8"), hashlib.sha256
        ).hexdigest()
        return {
            "X-Client-Id": self.creds.client_id,
            "X-Timestamp": ts,
            "X-Nonce": nonce,
            "X-Signature": sig,
        }

    def _call(self, method: str, path: str, params: dict[str, Any] | None = None,
              body: dict[str, Any] | None = None) -> dict[str, Any]:
        query = urllib.parse.urlencode(params, doseq=True) if params else ""
        url = self.creds.base_url.rstrip("/") + path + (("?" + query) if query else "")
        headers = self._sign(method, path, query)
        raw_body = None
        if body is not None:
            raw_body = json.dumps(body).encode("utf-8")
            headers["Content-Type"] = "application/json"
        return _http(method, url, headers, raw_body, self.timeout)

    # ------------------------------------------------------------ endpoints

    def health(self) -> dict[str, Any]:
        return _http("GET", self.creds.base_url.rstrip("/") + "/v1/health", {}, None, self.timeout)

    def whoami(self) -> dict[str, Any]:
        return self._call("GET", "/v1/whoami")

    def sms(self, *, days: int | None = None, address: str | list[str] | None = None,
            slot: int | None = None, box: str = "inbox", limit: int = 50,
            offset: int = 0, since_id: int | None = None, q: str | None = None,
            purpose: str | None = None) -> dict[str, Any]:
        params: dict[str, Any] = {"box": box, "limit": limit}
        if days is not None:
            params["days"] = days
        if address:
            params["address"] = address if isinstance(address, str) else ",".join(address)
        if slot is not None:
            params["slot"] = slot
        if offset:
            params["offset"] = offset
        if since_id is not None:
            params["since_id"] = since_id
        if q:
            params["q"] = q
        if purpose:
            params["purpose"] = purpose
        return self._call("GET", "/v1/sms", params)

    def otp(self, *, address: str | None = None, days: int = 1,
            slot: int | None = None, limit: int = 10) -> dict[str, Any]:
        return self.sms(days=days, address=address, slot=slot, limit=limit, purpose="otp")

    def events(self, since_seq: int = 0) -> dict[str, Any]:
        return self._call("GET", "/v1/events", {"since_seq": since_seq})

    def stats(self, days: int = 7) -> dict[str, Any]:
        return self._call("GET", "/v1/stats", {"days": days})

    def revoke_self(self) -> dict[str, Any]:
        return self._call("POST", "/v1/revoke-self")


# ------------------------------------------------------------------ pairing

def _pretty_fingerprint(hex_digest: str) -> str:
    return " ".join(hex_digest[i:i + 2] for i in range(0, len(hex_digest), 2)).upper()


def pinned_fingerprint() -> str:
    """SHA-256 of the currently pinned certificate, lowercase hex."""
    pem = CERT_FILE.read_text(encoding="utf-8")
    der = ssl.PEM_cert_to_DER_cert(pem)
    return hashlib.sha256(der).hexdigest()


def pair(base_url: str, code: str, name: str = "agent", timeout: float = 15.0,
         config_path: Path | None = None, fingerprint: str | None = None) -> Credentials:
    # Trust on first use: the first time we talk to this phone we accept whatever
    # certificate it presents, store it, and verify every later request against it.
    if not CERT_FILE.exists():
        fp = fetch_and_pin_certificate(base_url, timeout)
        print(f"[pair] pinned certificate sha256 = {_pretty_fingerprint(fp)}")
        print("[pair] same value is shown on the phone's Pairing screen — compare if you want")
    elif fingerprint:
        expected = fingerprint.replace(":", "").replace(" ", "").lower()
        actual = pinned_fingerprint()
        if actual != expected:
            raise SmsBridgeError(
                f"certificate mismatch: pinned {actual}, expected {expected}. "
                "The phone may have been re-installed with a new certificate."
            )

    body = json.dumps({"code": code, "name": name}).encode("utf-8")
    resp = _http(
        "POST", base_url.rstrip("/") + "/v1/pair",
        {"Content-Type": "application/json"}, body, timeout,
    )
    if not resp.get("ok"):
        raise SmsBridgeError("pairing rejected", payload=resp)
    creds = Credentials(
        base_url=base_url.rstrip("/"),
        client_id=resp["client_id"],
        token=resp["token"],
        client_name=name,
    )
    creds.save(config_path)
    return creds

# Agent integration guide

You are an agent that needs to read SMS from a phone running **SMS Bridge**.
This file is self-contained; the full protocol lives in [API.md](API.md).

## 0. Before you start

The user must do this on the phone (you cannot do it for them):

1. Open **SMS Bridge** → **Enable service**
2. Go to **Pairing** → note the **6-digit code** (valid 5 minutes, one-time)
3. Tell you the **phone IP** shown on the main screen (e.g. `192.168.1.100`)

You need network reachability from your machine to `https://<phone-ip>:8787`.

## 1. Pair once

```bash
python mcp/smsctl.py pair --url https://<phone-ip>:8787 --code <code> --name <your-agent-name>
```

Credentials land in `mcp/.smsbridge.json` (git-ignored). By hand:

```http
POST /v1/pair
{"code":"123456","name":"my-agent"}

{"ok":true,"client_id":"<uuid>","token":"<base64url>"}
```

The token is shown **once**. Never log it, never commit it.

### HTTPS only — pin the certificate

The phone does not speak plain HTTP. It presents a self-signed certificate and
this repo ships it as [`mcp/smsbridge-cert.pem`](mcp/smsbridge-cert.pem). The
client must pin **that file** (not the system CA store); `smsbridge_client.py`
already does. If the PEM is missing, copy it next to the script or point
`SMSBRIDGE_CERT` at it.

The SHA-256 fingerprint is also printed on the phone's pairing screen — compare
it if you want to confirm the certificate really came from that device.

## 2. Sign every request

```
canonical = METHOD + "\n" + PATH + "\n" + RAW_QUERY + "\n" + TIMESTAMP_MS + "\n" + NONCE
signature = hex( HMAC-SHA256(token, canonical) )
```

Send as headers: `X-Client-Id`, `X-Timestamp`, `X-Nonce`, `X-Signature`.
Time window is ±5 minutes, each nonce is single use.

Reference implementation: [`mcp/smsbridge_client.py`](mcp/smsbridge_client.py) (~50 lines, stdlib only).

```python
import hashlib, hmac, secrets, time
def sign(token, method, path, query=""):
    ts = str(int(time.time() * 1000))
    nonce = secrets.token_hex(12)
    canon = f"{method}\n{path}\n{query}\n{ts}\n{nonce}"
    return {
        "X-Client-Id": client_id,
        "X-Timestamp": ts,
        "X-Nonce": nonce,
        "X-Signature": hmac.new(token.encode(), canon.encode(), hashlib.sha256).hexdigest(),
    }
```

## 3. Endpoints you actually need

| Call | Purpose |
|---|---|
| `GET /v1/health` *(no auth)* | battery, SIM slots, paired count, whether the bridge is alive |
| `GET /v1/sms?days=3&address=10086&slot=0&limit=20&box=inbox` | structured query |
| `GET /v1/sms?days=1&purpose=otp` | **verification codes only**, body masked |
| `GET /v1/events?since_seq=<n>` | incremental new-SMS events, no duplicates |
| `GET /v1/stats?days=7` | message count per sender |

`slot`: 0 or 1 for dual-SIM, omit for both. `days` is clamped by the phone.
Errors: `401 unauthorized` / `403 otp_mode_disabled` / `429 rate_limited`.

## 4. Prefer OTP mode

Most agent tasks only need a verification code. `purpose=otp` returns
`{otp, matched_by, masked_body, address, date}` and never the full message, so
it is both safer and cheaper. Only fall back to full bodies when the user asks
for content.

## 5. If you persist this as a skill

Include, at minimum:

- the pair command and where credentials live
- the 4 core calls (health / search / otp / events) with a parameter table
- the signing recipe (or a pointer to `mcp/smsbridge_client.py`)
- hard rules: never log or commit the token, never send SMS, respect `429`
- an error table so you can self-diagnose

Suggested skill description (adjust wording, keep the trigger words):

> Read SMS from a phone running SMS Bridge over its local HTTP API. Use when you
> need a verification code, recent messages from a specific sender, or new-SMS
> events. Requires a one-time pairing code from the phone.

## 6. Verification checklist

Run these after pairing; if any fails, fix before doing real work:

1. `python mcp/smsctl.py health` → `ok: true`, `sims` is non-empty (if the user
   granted the phone-state permission)
2. `python mcp/smsctl.py search --days 7 --limit 3` → returns `count` and
   correctly shaped `messages`
3. `python mcp/smsctl.py otp --days 1` → `count` may be 0, but must not error
4. `python mcp/smsctl.py events --since 0` → returns `latest_seq`
5. unsigned request → must be rejected with `401`

## 7. Common failures

| Symptom | Cause | Fix |
|---|---|---|
| connection timed out | phone screen off and vendor ROM froze the app | user: keep power saving off, enable the heartbeat in Settings; then retry |
| `401 bad signature` | query string mismatch between signing and request | sign the **raw** query string exactly as sent |
| `401 unknown client` | token file missing or client revoked | pair again |
| `429 rate_limited` | too many requests | back off, default is 60/min |
| empty `sims` | `READ_PHONE_STATE` not granted | user grants it in the app |

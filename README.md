# SMS Bridge · SmsBridge

**English** | [简体中文](README.zh.md) | [日本語](README.ja.md)

Turn your phone's SMS into a local HTTP API that agents can query. The SMS database is only
touched when a request arrives — there is no polling loop.

```
┌─────────────┐    HTTP + HMAC-SHA256    ┌──────────────┐
│  Phone app  │ ◀──────────────────────▶ │    Agent     │
│ SMS Bridge  │    LAN / tunnel          │ CLI or MCP   │
└─────────────┘                          └──────────────┘
```

## Pair an Agent

> **You can let your agent do everything below for you. All you need is the SMS Bridge APK installed on your phone.**

> Requirement: the agent's machine must be able to reach port `8787` on the phone
> (same Wi-Fi, or a Tailscale / WireGuard tunnel).

**① On the phone** — open the app → **Enable service** → open **Pairing**

- the main screen shows the endpoint, e.g. `https://192.168.1.100:8787`
- the pairing screen shows a 6-digit code (**valid 5 minutes, one-time**)

**② On the agent side** — pair once

```powershell
python mcp\smsctl.py pair --url https://192.168.1.100:8787 --code 123456 --name my-agent
```

Credentials are written to `mcp\.smsbridge.json` (git-ignored); every later request is HMAC-signed.

**③ Verify**

```powershell
python mcp\smsctl.py health              # ok: true + both SIM slots
python mcp\smsctl.py search --days 3     # SMS from the last 3 days
python mcp\smsctl.py otp --days 1        # verification codes only, body masked
```

The Pairing screen lists every agent's last activity, request count and IP, and can revoke one
instantly. Integration details: [AGENT.md](AGENT.md). Protocol: [API.md](API.md).

## Install and first run

1. Install: `adb install -r app\build\outputs\apk\debug\app-debug.apk` (or grab it from Releases)
2. Open the app → **Enable service** → grant SMS / phone state / notification permissions
3. **Required on Chinese vendor ROMs** (measured: without these two steps the app is frozen
   40 seconds after the screen turns off, and a foreground service does not save it):
   - Settings → Apps → SMS Bridge → battery behaviour → 「allow all background activity」, allow auto-start
   - **Turn off power saving mode**
4. Open **Self-check & keep-alive** and fix every red item

There is also a 20-second heartbeat switch in Settings (on by default) as a fallback for other ROMs.

## Features

- **Structured queries**: filter by days / address / SIM slot (dual SIM) / keyword / inbox or sent, returns JSON
- **OTP mode**: `purpose=otp` returns only the verification code plus a masked body
- **Incremental events**: a ContentObserver queues new SMS in real time; agents pull with `since_seq`, no duplicates
- **Always on**: `specialUse` foreground service + two watchdogs + boot start; keeps answering with the screen locked or the app swiped away
- **Pairing & audit**: one-time 6-digit code, revocable clients, request audit log, rate limiting
- **Read-only**: never sends SMS, never writes the SMS database, never copies messages locally

## Keep-alive semantics (unless-stopped)

`serviceEnabled` records your intent:

- **Enable service** → true → the service comes back after a reboot, after being killed, or after swiping the app away
- **Stop** → false → it is never auto-started again

Four recovery paths: boot / app-update broadcast, 15-minute AlarmManager, 15-minute persisted
JobScheduler, and every time the app is opened.

**Only exception**: "Force stop" in system settings — Android marks the app as stopped and every
broadcast, alarm and job is dropped until the app is opened manually. That is a system-level
restriction and cannot be bypassed without root.

## Languages

The UI ships in **English / 简体中文 / 日本語**, switchable from the top-right button on the main
screen. It also registers a system-level per-app language (`Settings → Apps → SMS Bridge → Language`).
Follows the system by default; falls back to English.

## Building

Requires JDK 17+ / Android SDK 36 / Gradle 8.13. Full steps in [BUILD.md](BUILD.md); shortest path:

```bash
echo "sdk.dir=/path/to/Android/Sdk" > local.properties
python tools/gen_cert.py   # TLS keystore, once per clone
gradle assembleDebug       # -> app/build/outputs/apk/debug/app-debug.apk
```

> **For AI agents**: if the machine has no Android toolchain, **install it yourself** instead of
> asking the user — Temurin JDK 21 + Android command-line tools
> (`sdkmanager "platforms;android-36" "build-tools;36.0.0" "platform-tools"`) + Gradle 8.13,
> write `local.properties` with `sdk.dir`, then `gradle assembleDebug`.

## Security model

- **Transport is TLS.** The app serves HTTPS with a self-signed certificate. The agent trusts it on
  first pair (TOFU): the certificate is fetched and pinned automatically, then strictly verified —
  no file copying, no CA needed. A device on the same Wi-Fi cannot read SMS bodies or pairing tokens
  from a packet capture.
  Known limit: the private key ships inside the APK, so someone who also decompiles the APK could
  decrypt. That trade-off buys zero-config HTTPS for a self-hosted tool.
- Every agent gets its own token (32 random bytes); requests are HMAC-SHA256 signed with a ±5 minute
  window and nonce replay protection
- Hard limits live on the phone: max days, max messages, rate limit, blocked numbers — agents cannot change them
- Tokens are stored only in the app's private storage (unreadable by other apps on non-rooted devices);
  revocation is immediate
- For cross-network access still prefer Tailscale / WireGuard

## Docs

| File | Contents |
|---|---|
| [AGENT.md](AGENT.md) | Integration guide for AI agents (pairing, signing, checklist, error table) |
| [API.md](API.md) | REST API and signing spec |
| [BUILD.md](BUILD.md) | Build environment (Android Studio / CLI / agent auto-setup) |
| [mcp/](mcp) | Python CLI + MCP server |

## License

MIT

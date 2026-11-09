# Windows SMS Bridge

**English** | [简体中文](README.zh.md) | [日本語](README.ja.md)

- **Phone app download** → <https://github.com/tiny-andr/android-agent-sms-bridge/releases>
- **Windows build download** → <https://github.com/tiny-andr/win-sms-bridge/releases/latest>

A Windows desktop client for [android-agent-sms-bridge](https://github.com/tiny-andr/android-agent-sms-bridge): browse, search and auto-sync the messages that already sit on your phone, without uploading anything to a cloud.

```
┌────────────────┐   HTTPS + HMAC-SHA256   ┌──────────────────────┐
│  Android app   │ ◀─────────────────────▶ │  Windows SMS Bridge  │
│  SMS Bridge    │     same Wi-Fi / VPN    │  this app            │
└────────────────┘                         └──────────────────────┘
```

## Pair a phone

1. On the phone: open **SMS Bridge** → **Enable service** → note the address shown on the main screen, e.g. `https://192.168.1.100:8787`
2. On the phone: open **Pairing** and get the 6-digit code (valid for 5 minutes, single use)
3. On the PC: click **Pair phone** at the top right → enter the address and the code → **Pair & sync**
4. Compare the certificate fingerprint shown in the dialog with the one on the phone's pairing screen

The first connection pins the phone's self-signed certificate (TOFU); every later request is verified against it, so message bodies travel encrypted.

You can pair several phones. Each one gets its own key and appears as a chip in the device rail; revoke any of them from the phone's pairing list at any time.

> Upgrading the Android app changes its certificate, so an old key stops working: right-click the device → **Re-pair (new key / certificate)**. Cached history is kept.

## Features

| | |
|---|---|
| Multiple phones | Each phone keeps its own key, battery level, carrier and last-sync time |
| Dual SIM | Every message is tagged with the card that received it (SIM 1 · 2132); filter by card |
| Manual sync | **Sync** in the device rail (`Ctrl+R`) — syncs the selected phone, or all of them |
| Scheduled sync | Off / 1 / 5 / 15 / 30 / 60 minutes, quietly in the background |
| Start with Windows | Single `Run` entry, optionally start hidden in the tray |
| Tray resident | Closing the window hides it; the tray menu can sync and quit |
| Views | All / Inbox / Sent / Unread, per device or across all devices |
| Conversation view | Grouped by number, unread counts, day separators, in/out bubbles |
| Search | Fuzzy match on number or body (`Ctrl+K`) |
| Export | One conversation or everything, as CSV (UTF-8 BOM, opens cleanly in Excel) |
| Themes | Dark glass and warm paper, switched from the title bar |
| Display scale | 85 %–150 %, scales text and spacing together |
| Languages | English, 简体中文, 日本語 — follows the system by default |
| Read-only | Never writes to the phone: no sending, no marking as read |

## How syncing works

- **First sync** (right after pairing): reads the last 30 days with `GET /v1/sms?days=30&box=all&limit=200`, paging by offset.
- **Every other sync**: `GET /v1/sms?since_id=<highest local id>&box=all`, so it only pulls what is new.
- Messages are de-duplicated by the phone's own `_id`; if a body or read flag changes, the local copy is updated in place.
- One job per phone at a time. The phone allows 60 requests/minute; a normal sync is a single request.

The phone clamps `days` to 30 and `limit` to 200, so older history needs a higher limit on the phone side.

## Layout

Devices are the top level; views come second.

```
┌ Devices  [All devices] [Pixel 9] [Backup]                [Sync] [Pair phone] ┐
│                                                                              │
│  Views             Conversations                  Thread                     │
│  All messages                                                               │
│  Inbox / Sent      grouped by number              bubbles, day separators   │
│  Unread                                                                     │
└──────────────────────────────────────────────────────────────────────────────┘
```

## Build from source

Requires Node.js 18+.

```bash
npm install
npm start            # run from source
npm run pack         # portable folder, dist/Windows SMS Bridge-win32-x64/
npm run dist         # installer + portable zip, dist/
npm run icons        # regenerate the app icons (Python + Pillow)
```

`npm run dist` produces two artefacts:

| Artefact | Use |
|---|---|
| `Windows-SMS-Bridge-Setup-1.2.0.exe` | Per-user installer: Start Menu and desktop shortcuts, appears in *Apps & features*, no admin rights needed |
| `Windows SMS Bridge-1.2.0-win.zip` | Green/portable build: unzip anywhere and run the exe, nothing installed |

Both keep all data in `%APPDATA%\Windows SMS Bridge\`, so switching between them keeps your pairings.

## Data

Everything stays on this computer, in `%APPDATA%\Windows SMS Bridge\`:

- `config.json` — paired phones and preferences; the pairing token is encrypted with Windows DPAPI
- `messages.json` — the local message cache (plain JSON; protect the disk with BitLocker if needed)

The app is read-only: it never writes to the phone, never sends an SMS, and never marks anything as read on the device.

## Security model

- **Transport is TLS.** The phone serves HTTPS with a self-signed certificate; the first pairing pins it (TOFU) and every later request is verified against that exact certificate. Passive sniffing on the same Wi-Fi cannot read message bodies or the pairing token.
- **Signing on top of TLS.** Every request carries `X-Client-Id`, `X-Timestamp`, `X-Nonce` and an HMAC-SHA256 signature over method, path, raw query, timestamp and nonce — covering tampering and replay (±5 minute window, single-use nonce).
- **Known limitation.** The phone's private key ships inside the APK, so TLS stops passive eavesdroppers but not someone who can extract that key. Real authentication still comes from the token: without it a request cannot be signed.
- **Token at rest.** Stored with Windows DPAPI — only this Windows account can decrypt it.
- **Remote access.** Use Tailscale or WireGuard; never port-forward 8787 to the internet.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| Connection refused | The bridge service is stopped | Enable the service in the phone app |
| Timeout | Screen off and the vendor ROM froze the app | Allow background activity, disable battery optimisation, retry |
| "Certificate is not trusted" | The certificate changed (app reinstalled) or was never pinned | Re-pair the device |
| "Re-pair needed" on a device chip | The key was revoked on the phone | Right-click → **Re-pair** |
| Empty Inbox | Nothing in the last 30 days, or `READ_SMS` not granted | Check permissions and the history window on the phone |

## Related

- Android app and protocol documentation: [android-agent-sms-bridge](https://github.com/tiny-andr/android-agent-sms-bridge)

## License

MIT

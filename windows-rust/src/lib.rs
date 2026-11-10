//! Windows SMS Bridge — Tauri 2 client for the Android SMS bridge (短信桥).
//!
//! This crate replaces the Electron main process (`windows/main.js` + the
//! modules under `windows/src/core/`). The renderer is the unchanged
//! HTML/CSS/JS from `windows/src/ui`; only the `window.sms` transport changed
//! (preload.js → web/ui/tauri.js over Tauri IPC).
//!
//! Hard contracts, all inherited from the Electron build:
//!  · the same data directory, file names and JSON shapes
//!    (`%APPDATA%\Windows SMS Bridge\{config.json,messages.json}`)
//!  · the same 22 IPC channels, unchanged names and payloads (see `commands`)
//!  · TOFU certificate pinning over rustls, HMAC-SHA256 request signing,
//!    DPAPI-sealed pairing tokens (see `smsbridge`, `dpapi`)
//!  · read-only against the phone: no SMS is ever sent, no read flag is written

pub mod app;
pub mod autostart;
pub mod commands;
pub mod dpapi;
pub mod export;
pub mod i18n;
pub mod migrate;
pub mod notifications;
pub mod shell_ops;
pub mod smsbridge;
pub mod store;
pub mod sync;
pub mod tray;
pub mod updater;

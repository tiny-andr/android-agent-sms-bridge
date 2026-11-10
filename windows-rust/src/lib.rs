//! Windows SMS Bridge — protocol, storage and migration layer.
//!
//! This is the part of the Tauri rewrite that has no windowing dependency: the
//! phone protocol (rustls pinning + HMAC signing), the local JSON store, the
//! profile migration and the sync engine. It is exercised by unit tests and by
//! `tests/live_protocol.rs` (against a real phone).

pub mod dpapi;
pub mod i18n;
pub mod migrate;
pub mod smsbridge;
pub mod store;
pub mod sync;

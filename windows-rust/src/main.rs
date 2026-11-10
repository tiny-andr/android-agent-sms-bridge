//! Windows SMS Bridge — desktop entry point.
//!
//! All logic lives in the library (`src/lib.rs`) so integration tests can drive
//! the protocol and storage layers without a window.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // One process-wide rustls backend for every TLS client we build (pinned LAN
    // requests as well as the update checker).
    let _ = rustls::crypto::ring::default_provider().install_default();
    windows_sms_bridge::app::run();
}

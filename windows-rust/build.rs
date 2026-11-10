fn main() {
    // The renderer is plain files copied from the Electron build (no bundler),
    // so make sure cargo re-embeds `web/` whenever any of it changes.
    println!("cargo:rerun-if-changed=web/ui");
    println!("cargo:rerun-if-changed=web/shared");
    println!("cargo:rerun-if-changed=capabilities");
    println!("cargo:rerun-if-changed=tauri.conf.json");
    tauri_build::build()
}

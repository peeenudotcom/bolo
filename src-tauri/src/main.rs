// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Packaging check: exercise the OS loader without recording or opening UI.
    if std::env::args().any(|arg| arg == "--check-runtime") {
        println!("Bolo {} native runtime loaded", env!("CARGO_PKG_VERSION"));
        return;
    }
    bolo_lib::run()
}

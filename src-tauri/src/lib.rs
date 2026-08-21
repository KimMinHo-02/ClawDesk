//! ClawDesk Rust backend.
//!
//! Layering (top → bottom, one-way dependency):
//! `commands` (Phase 2+) → `application` → `domain` ports → `infrastructure`.

pub mod application;
pub mod commands;
pub mod domain;
pub mod error;
pub mod infrastructure;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::detect_environment,
            commands::install_openclaw
        ])
        .run(tauri::generate_context!())
        .expect("error while running ClawDesk");
}

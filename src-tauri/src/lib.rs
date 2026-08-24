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
            commands::install_openclaw,
            commands::models::list_providers,
            commands::models::get_provider,
            commands::models::save_provider,
            commands::models::delete_provider,
            commands::models::list_models,
            commands::models::get_default_model,
            commands::models::set_default_model,
            commands::models::get_reasoning_default,
            commands::models::set_reasoning_default,
            commands::models::set_provider_api_key,
            commands::models::delete_provider_api_key,
            commands::models::list_api_keys,
            commands::skills::list_skills,
            commands::skills::set_skill_enabled,
            commands::plugins::list_plugins,
            commands::plugins::set_plugin_enabled,
            commands::plugins::get_plugin_runtime
        ])
        .run(tauri::generate_context!())
        .expect("error while running ClawDesk");
}

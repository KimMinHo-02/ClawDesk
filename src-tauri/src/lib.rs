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
            commands::plugins::get_plugin_runtime,
            commands::tools::get_tool_policy,
            commands::tools::set_tool_profile,
            commands::tools::set_tool_allow,
            commands::tools::set_tool_deny,
            commands::tools::set_exec_mode,
            commands::security::list_security_profiles,
            commands::security::save_security_profile,
            commands::security::delete_security_profile,
            commands::security::apply_security_profile,
            commands::security::run_security_audit,
            commands::channels::get_channels,
            commands::channels::get_channel_config,
            commands::channels::set_channel_token,
            commands::channels::delete_channel_token,
            commands::channels::connect_channel,
            commands::channels::set_channel_enabled,
            commands::channels::set_dm_access,
            commands::channels::set_group_policy,
            commands::channels::list_pairing_requests,
            commands::channels::approve_pairing,
            commands::automations::get_automations,
            commands::automations::get_automation,
            commands::automations::create_automation,
            commands::automations::update_automation,
            commands::automations::set_automation_enabled,
            commands::automations::delete_automation,
            commands::diagnostics::get_gateway_status,
            commands::diagnostics::get_update_status,
            commands::diagnostics::get_agents,
            commands::diagnostics::get_logs,
            commands::node_update::update_node
        ])
        .run(tauri::generate_context!())
        .expect("error while running ClawDesk");
}

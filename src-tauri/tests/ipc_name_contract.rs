//! Phase 7.5 IPC command name contract.
//!
//! `#[tauri::command(rename = "...")]`이 등록하는 command name은
//! tauri-macros가 생성하는 `__tauri_command_name_<ident>!()` macro가
//! 반환하는 문자열 리터럴이다 (runtime invoke handler가 정확히
//! 이 문자열을 match). 이 테스트는 command 로직을 실행하지 않고
//! (1) 각 command의 실제 등록 이름이 frontend kebab-case 계약과
//! 일치하는지, (2) 계약 command 전체가 lib.rs `generate_handler!`
//! 에 등록되어 있는지를 검증한다.
//!
//! 계약의 frontend 측 single source of truth: `src/lib/tauri/index.ts`
//! 의 `COMMANDS` object (49개).

/// (generate_handler! 내 fn path, frontend kebab-case name) —
/// `src/lib/tauri/index.ts` COMMANDS와 1:1.
const IPC_CONTRACT: &[(&str, &str)] = &[
    ("commands::detect_environment", "detect-environment"),
    ("commands::install_openclaw", "install-openclaw"),
    ("commands::models::list_providers", "list-providers"),
    ("commands::models::get_provider", "get-provider"),
    ("commands::models::save_provider", "save-provider"),
    ("commands::models::delete_provider", "delete-provider"),
    ("commands::models::list_models", "list-models"),
    ("commands::models::get_default_model", "get-default-model"),
    ("commands::models::set_default_model", "set-default-model"),
    (
        "commands::models::get_reasoning_default",
        "get-reasoning-default",
    ),
    (
        "commands::models::set_reasoning_default",
        "set-reasoning-default",
    ),
    (
        "commands::models::set_provider_api_key",
        "set-provider-api-key",
    ),
    (
        "commands::models::delete_provider_api_key",
        "delete-provider-api-key",
    ),
    ("commands::models::list_api_keys", "list-api-keys"),
    ("commands::skills::list_skills", "list-skills"),
    ("commands::skills::set_skill_enabled", "set-skill-enabled"),
    ("commands::plugins::list_plugins", "list-plugins"),
    (
        "commands::plugins::set_plugin_enabled",
        "set-plugin-enabled",
    ),
    (
        "commands::plugins::get_plugin_runtime",
        "get-plugin-runtime",
    ),
    ("commands::tools::get_tool_policy", "get-tool-policy"),
    ("commands::tools::set_tool_profile", "set-tool-profile"),
    ("commands::tools::set_tool_allow", "set-tool-allow"),
    ("commands::tools::set_tool_deny", "set-tool-deny"),
    ("commands::tools::set_exec_mode", "set-exec-mode"),
    (
        "commands::security::list_security_profiles",
        "list-security-profiles",
    ),
    (
        "commands::security::save_security_profile",
        "save-security-profile",
    ),
    (
        "commands::security::delete_security_profile",
        "delete-security-profile",
    ),
    (
        "commands::security::apply_security_profile",
        "apply-security-profile",
    ),
    (
        "commands::security::run_security_audit",
        "run-security-audit",
    ),
    ("commands::channels::get_channels", "get-channels"),
    (
        "commands::channels::get_channel_config",
        "get-channel-config",
    ),
    ("commands::channels::set_channel_token", "set-channel-token"),
    (
        "commands::channels::delete_channel_token",
        "delete-channel-token",
    ),
    ("commands::channels::connect_channel", "connect-channel"),
    (
        "commands::channels::set_channel_enabled",
        "set-channel-enabled",
    ),
    ("commands::channels::set_dm_access", "set-dm-access"),
    ("commands::channels::set_group_policy", "set-group-policy"),
    (
        "commands::channels::list_pairing_requests",
        "list-pairing-requests",
    ),
    ("commands::channels::approve_pairing", "approve-pairing"),
    ("commands::automations::get_automations", "get-automations"),
    ("commands::automations::get_automation", "get-automation"),
    (
        "commands::automations::create_automation",
        "create-automation",
    ),
    (
        "commands::automations::update_automation",
        "update-automation",
    ),
    (
        "commands::automations::set_automation_enabled",
        "set-automation-enabled",
    ),
    (
        "commands::automations::delete_automation",
        "delete-automation",
    ),
    (
        "commands::diagnostics::get_gateway_status",
        "get-gateway-status",
    ),
    (
        "commands::diagnostics::get_update_status",
        "get-update-status",
    ),
    ("commands::diagnostics::get_agents", "get-agents"),
    ("commands::diagnostics::get_logs", "get-logs"),
];

/// 각 command fn ident의 tauri-macros `__tauri_command_name_<ident>!()` —
/// runtime invoke handler가 match하는 실제 등록 이름.
fn registered_names() -> Vec<&'static str> {
    vec![
        clawdesk_lib::__tauri_command_name_detect_environment!(),
        clawdesk_lib::__tauri_command_name_install_openclaw!(),
        clawdesk_lib::__tauri_command_name_list_providers!(),
        clawdesk_lib::__tauri_command_name_get_provider!(),
        clawdesk_lib::__tauri_command_name_save_provider!(),
        clawdesk_lib::__tauri_command_name_delete_provider!(),
        clawdesk_lib::__tauri_command_name_list_models!(),
        clawdesk_lib::__tauri_command_name_get_default_model!(),
        clawdesk_lib::__tauri_command_name_set_default_model!(),
        clawdesk_lib::__tauri_command_name_get_reasoning_default!(),
        clawdesk_lib::__tauri_command_name_set_reasoning_default!(),
        clawdesk_lib::__tauri_command_name_set_provider_api_key!(),
        clawdesk_lib::__tauri_command_name_delete_provider_api_key!(),
        clawdesk_lib::__tauri_command_name_list_api_keys!(),
        clawdesk_lib::__tauri_command_name_list_skills!(),
        clawdesk_lib::__tauri_command_name_set_skill_enabled!(),
        clawdesk_lib::__tauri_command_name_list_plugins!(),
        clawdesk_lib::__tauri_command_name_set_plugin_enabled!(),
        clawdesk_lib::__tauri_command_name_get_plugin_runtime!(),
        clawdesk_lib::__tauri_command_name_get_tool_policy!(),
        clawdesk_lib::__tauri_command_name_set_tool_profile!(),
        clawdesk_lib::__tauri_command_name_set_tool_allow!(),
        clawdesk_lib::__tauri_command_name_set_tool_deny!(),
        clawdesk_lib::__tauri_command_name_set_exec_mode!(),
        clawdesk_lib::__tauri_command_name_list_security_profiles!(),
        clawdesk_lib::__tauri_command_name_save_security_profile!(),
        clawdesk_lib::__tauri_command_name_delete_security_profile!(),
        clawdesk_lib::__tauri_command_name_apply_security_profile!(),
        clawdesk_lib::__tauri_command_name_run_security_audit!(),
        clawdesk_lib::__tauri_command_name_get_channels!(),
        clawdesk_lib::__tauri_command_name_get_channel_config!(),
        clawdesk_lib::__tauri_command_name_set_channel_token!(),
        clawdesk_lib::__tauri_command_name_delete_channel_token!(),
        clawdesk_lib::__tauri_command_name_connect_channel!(),
        clawdesk_lib::__tauri_command_name_set_channel_enabled!(),
        clawdesk_lib::__tauri_command_name_set_dm_access!(),
        clawdesk_lib::__tauri_command_name_set_group_policy!(),
        clawdesk_lib::__tauri_command_name_list_pairing_requests!(),
        clawdesk_lib::__tauri_command_name_approve_pairing!(),
        clawdesk_lib::__tauri_command_name_get_automations!(),
        clawdesk_lib::__tauri_command_name_get_automation!(),
        clawdesk_lib::__tauri_command_name_create_automation!(),
        clawdesk_lib::__tauri_command_name_update_automation!(),
        clawdesk_lib::__tauri_command_name_set_automation_enabled!(),
        clawdesk_lib::__tauri_command_name_delete_automation!(),
        clawdesk_lib::__tauri_command_name_get_gateway_status!(),
        clawdesk_lib::__tauri_command_name_get_update_status!(),
        clawdesk_lib::__tauri_command_name_get_agents!(),
        clawdesk_lib::__tauri_command_name_get_logs!(),
    ]
}

#[test]
fn ipc_command_names_match_frontend_kebab_contract() {
    let names = registered_names();
    assert_eq!(names.len(), IPC_CONTRACT.len(), "contract size drift");
    for ((fn_path, kebab), actual) in IPC_CONTRACT.iter().zip(&names) {
        assert_eq!(actual, kebab, "registered IPC name mismatch for {fn_path}");
    }
}

#[test]
fn all_contract_commands_registered_in_invoke_handler() {
    let lib_src = include_str!("../src/lib.rs");
    for (fn_path, kebab) in IPC_CONTRACT {
        assert!(
            lib_src.contains(fn_path),
            "generate_handler! missing {fn_path} ({kebab})"
        );
    }
}

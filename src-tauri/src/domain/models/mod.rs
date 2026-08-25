//! Domain models (value types shared across layers).

pub mod channels;
pub mod install;
// `models` under `models` reads redundantly, but renaming would churn every
// import path; the inception lint is suppressed for this one module.
#[allow(clippy::module_inception)]
pub mod models;
pub mod openclaw;
pub mod skills;
pub mod tools;
pub mod windows;

pub use channels::{
    channel_allow_from_path, channel_dm_policy_path, channel_enabled_path,
    channel_group_policy_path, channel_secret_key_id, channel_secret_ref, channel_section_path,
    channel_token_path, classify_channel_token_state, parse_channel_config, parse_channel_list_row,
    parse_channel_status, parse_pairing_requests, validate_allow_from_entry, validate_channel_id,
    validate_channel_token, validate_dm_access, validate_dm_policy, validate_group_policy,
    validate_pairing_code, ChannelConfig, ChannelRow, ChannelStatus, ChannelStatusRow,
    ChannelSummary, ChannelTokenState, ChannelTokenStatus, ChannelsOverview, DISCORD_PLUGIN_ID,
    SUPPORTED_CHANNELS,
};
pub use install::{NpmEntry, ResolvedOpenClawEntry};
pub use models::{
    clawdesk_secret_ref, secret_key_id, validate_base_url, validate_input_modalities,
    validate_model_entry, validate_model_id, validate_model_ref, validate_provider,
    validate_provider_id, ApiKeyStatus, ModelCompat, ModelEntry, ModelRow, ProviderApiKey,
    ProviderDetail, ProviderPayload, ProviderSummary, SecretRef, ThinkingLevel,
    CLAWDESK_SECRET_ALIAS, KNOWN_API_TYPES,
};
pub use openclaw::{
    ExecutableDetection, GatewayStatus, OpenClawStatus, OpenClawVersion, UpdateState,
};
pub use skills::{validate_plugin_id, validate_skill_name, PluginRow, PluginRuntime, SkillRow};
pub use tools::{
    builtin_profiles, find_matching_profile, is_builtin_profile_id, parse_audit_document,
    parse_findings, parse_tool_policy, suppressed_count, validate_exec_mode, validate_profile,
    validate_profile_name, validate_profile_slug, validate_tool_entry, validate_tool_profile,
    SecurityAuditResult, SecurityFinding, SecurityProfile, ToolPolicy,
};
pub use windows::{Architecture, NodeDetection, WindowsVersion};

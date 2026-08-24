//! Domain models (value types shared across layers).

pub mod install;
// `models` under `models` reads redundantly, but renaming would churn every
// import path; the inception lint is suppressed for this one module.
#[allow(clippy::module_inception)]
pub mod models;
pub mod openclaw;
pub mod skills;
pub mod windows;

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
pub use windows::{Architecture, NodeDetection, WindowsVersion};

//! Domain types for the Phase 3 model/provider/API-key/reasoning feature.
//!
//! Wire shapes mirror the latest-stable OpenClaw config format (camelCase
//! field names such as `baseUrl`, `contextWindow`, `supportedReasoningEfforts`),
//! so provider/model payloads round-trip unchanged between UI and config.

use crate::error::AppError;

/// The standard OpenClaw thinking-level ladder (`tools/thinking` docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThinkingLevel {
    Off,
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
    Adaptive,
    Max,
    Ultra,
}

impl ThinkingLevel {
    /// All levels in the standard ladder order.
    pub const ALL: [ThinkingLevel; 9] = [
        Self::Off,
        Self::Minimal,
        Self::Low,
        Self::Medium,
        Self::High,
        Self::XHigh,
        Self::Adaptive,
        Self::Max,
        Self::Ultra,
    ];

    /// Parses a wire id (`"off"`, `"xhigh"`, ...). Case-sensitive, exact ids.
    pub fn parse(id: &str) -> Option<ThinkingLevel> {
        Self::ALL
            .iter()
            .find(|level| level.wire_id() == id)
            .copied()
    }

    /// The stable wire id of the level.
    pub fn wire_id(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Adaptive => "adaptive",
            Self::Max => "max",
            Self::Ultra => "ultra",
        }
    }
}

/// The provider's `apiKey` field as observed in the (redacted) config read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderApiKey {
    /// No `apiKey` in the provider config.
    #[default]
    Absent,
    /// A ClawDesk-managed exec SecretRef (value never exposed).
    Managed,
    /// Some other value is present (plaintext or foreign ref — never exposed).
    Other,
}

/// Reasoning-capability metadata from the model `compat` block.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCompat {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub supports_reasoning_effort: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supported_reasoning_efforts: Option<Vec<ThinkingLevel>>,
}

/// A model entry in a provider (config-format shape).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelEntry {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default = "default_input")]
    pub input: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compat: Option<ModelCompat>,
}

fn default_input() -> Vec<String> {
    vec!["text".to_string()]
}

/// A provider entry as read from / written to `models.providers.<id>`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDetail {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
    /// How the provider's `apiKey` field is currently populated (never the value).
    #[serde(rename = "apiKeyState", default, skip_serializing_if = "is_key_absent")]
    pub api_key: ProviderApiKey,
    #[serde(default)]
    pub models: Vec<ModelEntry>,
}

fn is_key_absent(state: &ProviderApiKey) -> bool {
    *state == ProviderApiKey::Absent
}

/// Provider payload sent to `config set` (config-format shape).
///
/// `api_key` carries the ClawDesk exec SecretRef when one must be (re)attached;
/// a plaintext value is never a payload field (S7).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<SecretRef>,
    #[serde(default)]
    pub models: Vec<ModelEntry>,
}

/// An OpenClaw SecretRef object (exec source, as used by ClawDesk).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretRef {
    pub source: String,
    pub provider: String,
    pub id: String,
}

/// One row of `openclaw models list --json` (fixture-defined contract shape).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRow {
    pub provider: String,
    pub model: String,
    /// `provider/model` reference.
    pub full: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supported_reasoning_efforts: Option<Vec<ThinkingLevel>>,
}

/// Provider list summary (computed view for the UI).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSummary {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
    pub model_count: usize,
    /// True only when the config holds a ClawDesk SecretRef **and** the key
    /// exists in the ClawDesk secret store.
    pub api_key_registered: bool,
}

/// API key registration state for one provider (ClawDesk secret store view).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyStatus {
    pub provider_id: String,
    pub registered: bool,
}

/// The exec provider alias ClawDesk registers in `secrets.providers`.
pub const CLAWDESK_SECRET_ALIAS: &str = "clawdesk";

/// Builds the ClawDesk-managed key id for a provider:
/// `providers/<providerId>/apiKey` (valid exec-id pattern).
pub fn secret_key_id(provider_id: &str) -> String {
    format!("providers/{provider_id}/apiKey")
}

/// Builds the ClawDesk exec SecretRef for a provider's API key.
pub fn clawdesk_secret_ref(provider_id: &str) -> SecretRef {
    SecretRef {
        source: "exec".to_string(),
        provider: CLAWDESK_SECRET_ALIAS.to_string(),
        id: secret_key_id(provider_id),
    }
}

// --- Input validation (S2: validate before any argv/config-path use) ---------

/// Valid provider/model id: starts alphanumeric, then `[A-Za-z0-9._-]`,
/// max 128 chars. Excludes `/`, `:`, whitespace, and `..` traversal.
fn validate_entry_id(id: &str, which: &str, invalid_code: &'static str) -> Result<(), AppError> {
    let bytes = id.as_bytes();
    let ok = match bytes.first() {
        Some(c) => c.is_ascii_alphanumeric(),
        None => false,
    } && bytes.len() <= 128
        && bytes
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
        && !id.contains("..");
    if ok {
        Ok(())
    } else {
        Err(AppError::invalid_input(invalid_code, which, id))
    }
}

/// Validates a provider id (used in config paths and key ids).
pub fn validate_provider_id(id: &str) -> Result<(), AppError> {
    validate_entry_id(id, "provider id", "provider-id-invalid")
}

/// Validates a model id (used in config paths and model refs).
pub fn validate_model_id(id: &str) -> Result<(), AppError> {
    validate_entry_id(id, "model id", "model-id-invalid")
}

/// Validates a `provider/model` reference: both parts must be valid ids.
pub fn validate_model_ref(model_ref: &str) -> Result<(), AppError> {
    let (provider, model) = match model_ref.split_once('/') {
        Some((p, m)) => (p, m),
        None => {
            return Err(AppError::invalid_input(
                "model-id-invalid",
                "model reference",
                model_ref,
            ))
        }
    };
    validate_provider_id(provider)?;
    validate_model_id(model)?;
    Ok(())
}

/// Validates a provider base URL: absolute `http://`/`https://` with a
/// non-empty host and no whitespace or control characters.
pub fn validate_base_url(url: &str) -> Result<(), AppError> {
    let (scheme, rest) = match url.split_once("://") {
        Some(parts) => parts,
        None => {
            return Err(AppError::invalid_input(
                "provider-id-invalid",
                "baseUrl",
                url,
            ))
        }
    };
    let ok_scheme = matches!(scheme, "http" | "https");
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let ok = ok_scheme
        && !host.is_empty()
        && url
            .bytes()
            .all(|b| b.is_ascii() && !b.is_ascii_whitespace() && b != 0x7f);
    if ok {
        Ok(())
    } else {
        Err(AppError::invalid_input(
            "provider-id-invalid",
            "baseUrl",
            url,
        ))
    }
}

/// Validates a model input-modality list: non-empty, subset of `text`/`image`.
pub fn validate_input_modalities(input: &[String]) -> Result<(), AppError> {
    let ok = !input.is_empty() && input.iter().all(|m| matches!(m.as_str(), "text" | "image"));
    if ok {
        Ok(())
    } else {
        Err(AppError::invalid_input(
            "model-id-invalid",
            "input modalities",
            "",
        ))
    }
}

/// Validates one model entry's user input.
pub fn validate_model_entry(model: &ModelEntry) -> Result<(), AppError> {
    validate_model_id(&model.id)?;
    validate_input_modalities(&model.input)?;
    if let Some(compat) = &model.compat {
        if let Some(efforts) = &compat.supported_reasoning_efforts {
            if efforts.is_empty() {
                return Err(AppError::invalid_input(
                    "model-id-invalid",
                    "supportedReasoningEfforts",
                    "",
                ));
            }
        }
    }
    Ok(())
}

/// Validates a full provider input (before any process run).
pub fn validate_provider(
    provider_id: &str,
    base_url: Option<&str>,
    api: &str,
    models: &[ModelEntry],
) -> Result<(), AppError> {
    validate_provider_id(provider_id)?;
    if let Some(url) = base_url {
        validate_base_url(url)?;
    }
    if !KNOWN_API_TYPES.contains(&api) {
        return Err(AppError::invalid_input(
            "provider-id-invalid",
            "api type",
            api,
        ));
    }
    let mut seen: Vec<&str> = Vec::new();
    for model in models {
        validate_model_entry(model)?;
        if seen.contains(&model.id.as_str()) {
            return Err(AppError::invalid_input(
                "model-id-invalid",
                "duplicate model id",
                &model.id,
            ));
        }
        seen.push(&model.id);
    }
    Ok(())
}

/// The request-adapter ids from the latest-stable docs.
pub const KNOWN_API_TYPES: [&str; 10] = [
    "openai-completions",
    "openai-responses",
    "openai-chatgpt-responses",
    "anthropic-messages",
    "google-generative-ai",
    "google-vertex",
    "github-copilot",
    "bedrock-converse-stream",
    "ollama",
    "azure-openai-responses",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str) -> ModelEntry {
        ModelEntry {
            id: id.to_string(),
            name: None,
            reasoning: false,
            input: vec!["text".to_string()],
            context_window: None,
            max_tokens: None,
            compat: None,
        }
    }

    // --- ThinkingLevel -------------------------------------------------------

    #[test]
    fn thinking_level_parses_all_wire_ids() {
        for level in ThinkingLevel::ALL {
            assert_eq!(ThinkingLevel::parse(level.wire_id()), Some(level));
        }
    }

    #[test]
    fn thinking_level_rejects_unknown_and_case_variants() {
        assert_eq!(ThinkingLevel::parse("very-high"), None);
        assert_eq!(ThinkingLevel::parse("HIGH"), None);
        assert_eq!(ThinkingLevel::parse(""), None);
        assert_eq!(ThinkingLevel::parse("x-high"), None);
    }

    // --- key id / secret ref ---------------------------------------------------

    #[test]
    fn secret_key_id_shape() {
        assert_eq!(secret_key_id("acme"), "providers/acme/apiKey");
    }

    #[test]
    fn clawdesk_secret_ref_shape() {
        let reference = clawdesk_secret_ref("acme");
        assert_eq!(reference.source, "exec");
        assert_eq!(reference.provider, "clawdesk");
        assert_eq!(reference.id, "providers/acme/apiKey");
    }

    // --- id validation -----------------------------------------------------------

    #[test]
    fn provider_id_accepts_normal_ids() {
        for id in [
            "acme",
            "a",
            "My-Provider",
            "my.provider",
            "p_1",
            "a1234567890123456789",
        ] {
            assert!(validate_provider_id(id).is_ok(), "{id}");
        }
    }

    #[test]
    fn provider_id_rejects_traversal_and_injection() {
        for id in [
            "",
            "..",
            "../evil",
            "a/b",
            "a:b",
            "a b",
            "a;b",
            "$(rm -rf)",
            "a\nb",
            &"x".repeat(129),
            ".hidden",
        ] {
            let err = validate_provider_id(id).expect_err(&format!("{id:?} must be rejected"));
            assert_eq!(err.code, "provider-id-invalid", "{id:?}");
        }
    }

    #[test]
    fn model_ref_requires_valid_both_parts() {
        assert!(validate_model_ref("acme/gpt-1").is_ok());
        for bad in [
            "acme",
            "acme/",
            "/gpt-1",
            "../gpt-1",
            "ac/me/gpt-1",
            "acme/a:b",
        ] {
            assert!(validate_model_ref(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn base_url_validation() {
        assert!(validate_base_url("https://api.example.com/v1").is_ok());
        assert!(validate_base_url("http://localhost:4000/v1").is_ok());
        for bad in [
            "api.example.com",
            "ftp://x.com",
            "https://",
            "https://a b.com",
            "https://a\r.com",
            "https://a\nb.com",
            "",
        ] {
            assert!(validate_base_url(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn input_modality_validation() {
        assert!(validate_input_modalities(&["text".into()]).is_ok());
        assert!(validate_input_modalities(&["text".into(), "image".into()]).is_ok());
        assert!(validate_input_modalities(&[]).is_err());
        assert!(validate_input_modalities(&["audio".into()]).is_err());
    }

    #[test]
    fn provider_validation_covers_all_inputs() {
        assert!(
            validate_provider("acme", Some("https://x.com"), "openai-completions", &[]).is_ok()
        );
        // api type
        assert!(matches!(
            validate_provider("acme", Some("https://x.com"), "not-an-api", &[]),
            Err(err) if err.code == "provider-id-invalid"
        ));
        // base url
        assert!(matches!(
            validate_provider("acme", Some("ftp://x.com"), "openai-completions", &[]),
            Err(err) if err.code == "provider-id-invalid"
        ));
        // model id
        let bad_model = model("../evil");
        assert!(matches!(
            validate_provider("acme", None, "openai-completions", &[bad_model]),
            Err(err) if err.code == "model-id-invalid"
        ));
        // duplicate model id
        assert!(matches!(
            validate_provider("acme", None, "openai-completions", &[model("a"), model("a")]),
            Err(err) if err.code == "model-id-invalid"
        ));
    }
}

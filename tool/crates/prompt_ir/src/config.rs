//! Provider and analysis configuration types.

use serde::{Deserialize, Serialize};

/// Supported LLM provider types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ProviderKind {
    OpenAI,
    Anthropic,
    CustomHttp,
    Local,
    /// Ollama Cloud: hosted open models behind the OpenAI-compatible
    /// endpoint at `https://ollama.com/v1`, authenticated with `OLLAMA_API_KEY`.
    OllamaCloud,
}

impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderKind::OpenAI => write!(f, "OpenAI"),
            ProviderKind::Anthropic => write!(f, "Anthropic"),
            ProviderKind::CustomHttp => write!(f, "CustomHttp"),
            ProviderKind::Local => write!(f, "Local"),
            ProviderKind::OllamaCloud => write!(f, "OllamaCloud"),
        }
    }
}

/// Runtime provider configuration.
///
/// Resolution order: debug launch config → per-file annotations →
/// workspace `promptdbg.toml` → editor settings → auto-detection from environment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub provider: ProviderKind,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

fn default_timeout() -> u64 {
    30000
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            provider: ProviderKind::OpenAI,
            model: "gpt-4".to_string(),
            endpoint: None,
            api_key: None,
            api_key_env: Some("OPENAI_API_KEY".to_string()),
            timeout_ms: default_timeout(),
        }
    }
}

/// Analysis provider configuration (for IR generation, linting, tests).
///
/// If not provided, inherits from `ProviderConfig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisConfig {
    pub provider: ProviderKind,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub temperature: f32,
    #[serde(default = "default_analysis_timeout")]
    pub timeout_ms: u64,
}

/// Wall-clock budget for a single analysis request, in milliseconds.
///
/// The default is 60s and was, until this became configurable, the only value:
/// a request slower than it surfaced as a connection failure, with nothing to
/// indicate a deadline had passed. That is a poor default for current models,
/// many of which emit a long reasoning trace before their answer and
/// legitimately need more than a minute on a large prompt. Silently treating
/// "slow" as "unavailable" is how a property of this client can be mistaken for
/// a property of a model family.
///
/// Override with `PROMPTDBG_ANALYSIS_TIMEOUT_MS` (accepted range 1s to 30min).
/// The default is deliberately unchanged so that previously published
/// measurements remain reproducible.
fn default_analysis_timeout() -> u64 {
    std::env::var("PROMPTDBG_ANALYSIS_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|ms| (1_000..=1_800_000).contains(ms))
        .unwrap_or(60_000)
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            provider: ProviderKind::OpenAI,
            model: "gpt-4o-mini".to_string(),
            api_key: None,
            api_key_env: Some("OPENAI_API_KEY".to_string()),
            endpoint: None,
            temperature: 0.0,
            timeout_ms: default_analysis_timeout(),
        }
    }
}

impl From<&ProviderConfig> for AnalysisConfig {
    fn from(config: &ProviderConfig) -> Self {
        Self {
            provider: config.provider,
            model: config.model.clone(),
            api_key: config.api_key.clone(),
            api_key_env: config.api_key_env.clone(),
            endpoint: config.endpoint.clone(),
            temperature: 0.0,
            // Inherit the (usually larger) provider timeout; cloud-hosted open models
            // can take well over the analysis default to return a full IR.
            timeout_ms: config.timeout_ms.max(default_analysis_timeout()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_config_default() {
        let config = ProviderConfig::default();
        assert_eq!(config.provider, ProviderKind::OpenAI);
        assert_eq!(config.model, "gpt-4");
        assert_eq!(config.timeout_ms, 30000);
    }

    #[test]
    fn test_analysis_config_from_provider() {
        let provider = ProviderConfig {
            provider: ProviderKind::Anthropic,
            model: "claude-sonnet-4-20250514".to_string(),
            api_key_env: Some("ANTHROPIC_API_KEY".to_string()),
            ..Default::default()
        };
        let analysis = AnalysisConfig::from(&provider);
        assert_eq!(analysis.provider, ProviderKind::Anthropic);
        assert_eq!(analysis.temperature, 0.0);
    }

    #[test]
    fn test_provider_kind_serialization() {
        let kind = ProviderKind::OpenAI;
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(json, "\"OpenAI\"");
    }
}

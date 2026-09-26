//! Provider configuration and auto-detection for PromptDbg.
//!
//! This crate handles:
//! - Loading configuration from multiple sources (files, env, annotations)
//! - Auto-detecting providers from environment variables
//! - Resolving and validating the final configuration

use std::env;
use std::path::Path;

use prompt_ir::{AnalysisConfig, ProviderConfig, ProviderKind};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("No provider configured.\n\nTo fix this, set one of these environment variables:\n  export OPENAI_API_KEY=\"sk-...\"\n  export ANTHROPIC_API_KEY=\"sk-ant-...\"\n  export OLLAMA_API_KEY=\"...\"  # Ollama Cloud (open models)")]
    NoProvider,

    #[error(
        "Missing required field: {field}\n\nTo fix this, add '{field}' to your configuration."
    )]
    MissingField { field: &'static str },

    #[error("Invalid configuration value for {field}: {value}\n\nExpected: {hint}")]
    InvalidValue {
        field: &'static str,
        value: String,
        hint: String,
    },

    #[error("Failed to read config file: {0}")]
    FileError(#[from] std::io::Error),

    #[error("Failed to parse config file: {message}\n\nCheck your TOML syntax at {location}")]
    ParseError { message: String, location: String },

    #[error("API key not found: {env_var}\n\nTo fix this, set the environment variable:\n  export {env_var}=\"your-api-key\"")]
    ApiKeyNotFound { env_var: String },

    #[error("Custom HTTP provider requires an endpoint URL.\n\nTo fix this, add 'endpoint' to your configuration:\n  endpoint = \"https://your-api.example.com/v1/chat/completions\"")]
    MissingEndpoint,

    #[error("Invalid timeout value: {value}ms (must be between 1000 and 300000)\n\nTo fix this, use a value like:\n  timeout_ms = 30000")]
    InvalidTimeout { value: u64 },

    #[error("Unknown provider: {provider}\n\nSupported providers: openai, anthropic, custom_http, local, ollama_cloud")]
    UnknownProvider { provider: String },
}

/// Validation result with warnings.
#[derive(Debug)]
pub struct ValidationResult {
    pub config: ProviderConfig,
    pub warnings: Vec<String>,
}

/// Validate a provider configuration.
pub fn validate_config(config: &ProviderConfig) -> Result<ValidationResult, ConfigError> {
    let mut warnings = Vec::new();

    // Check for required fields based on provider type
    match config.provider {
        ProviderKind::CustomHttp => {
            if config.endpoint.is_none() {
                return Err(ConfigError::MissingEndpoint);
            }
        }
        ProviderKind::Local => {
            // Local doesn't need API key
            if config.endpoint.is_none() {
                warnings.push(
                    "No endpoint specified for local provider, using http://localhost:11434"
                        .to_string(),
                );
            }
        }
        ProviderKind::OpenAI | ProviderKind::Anthropic | ProviderKind::OllamaCloud => {
            // Check API key availability
            if config.api_key.is_none() && config.api_key_env.is_none() {
                let env_var = match config.provider {
                    ProviderKind::OpenAI => "OPENAI_API_KEY",
                    ProviderKind::Anthropic => "ANTHROPIC_API_KEY",
                    ProviderKind::OllamaCloud => "OLLAMA_API_KEY",
                    _ => "API_KEY",
                };
                return Err(ConfigError::ApiKeyNotFound {
                    env_var: env_var.to_string(),
                });
            }

            // Verify the API key is actually set
            if let Some(env_var) = &config.api_key_env {
                if env::var(env_var).is_err() {
                    return Err(ConfigError::ApiKeyNotFound {
                        env_var: env_var.clone(),
                    });
                }
            }
        }
    }

    // Validate timeout
    if config.timeout_ms < 1000 || config.timeout_ms > 300000 {
        return Err(ConfigError::InvalidTimeout {
            value: config.timeout_ms,
        });
    }

    // Warn about common issues
    if config.model.is_empty() {
        warnings.push("No model specified, will use provider default".to_string());
    }

    Ok(ValidationResult {
        config: config.clone(),
        warnings,
    })
}

/// Auto-detection configuration for providers.
struct AutoDetect {
    env_var: &'static str,
    provider: ProviderKind,
    default_model: &'static str,
}

const AUTO_DETECT_ORDER: &[AutoDetect] = &[
    AutoDetect {
        env_var: "OPENAI_API_KEY",
        provider: ProviderKind::OpenAI,
        default_model: "gpt-4",
    },
    AutoDetect {
        env_var: "ANTHROPIC_API_KEY",
        provider: ProviderKind::Anthropic,
        default_model: "claude-sonnet-4-20250514",
    },
    AutoDetect {
        env_var: "OLLAMA_API_KEY",
        provider: ProviderKind::OllamaCloud,
        default_model: "gpt-oss:120b",
    },
];

/// Attempt to auto-detect a provider from environment variables.
///
/// Model selection priority:
/// 1. `OPENAI_MODEL` or `ANTHROPIC_MODEL` env var (if set)
/// 2. Default model for the provider
pub fn auto_detect_provider() -> Option<ProviderConfig> {
    for detect in AUTO_DETECT_ORDER {
        if env::var(detect.env_var).is_ok() {
            // Check for model override env var (e.g., OPENAI_MODEL, ANTHROPIC_MODEL)
            let model_env_var = detect.env_var.replace("_API_KEY", "_MODEL");
            let model =
                env::var(&model_env_var).unwrap_or_else(|_| detect.default_model.to_string());

            return Some(ProviderConfig {
                provider: detect.provider,
                model,
                endpoint: None,
                api_key: None,
                api_key_env: Some(detect.env_var.to_string()),
                timeout_ms: 30000,
            });
        }
    }
    None
}

/// Resolve the API key from config (literal or environment variable).
pub fn resolve_api_key(config: &ProviderConfig) -> Result<String, ConfigError> {
    if let Some(key) = &config.api_key {
        return Ok(key.clone());
    }

    if let Some(env_var) = &config.api_key_env {
        return env::var(env_var).map_err(|_| ConfigError::ApiKeyNotFound {
            env_var: env_var.clone(),
        });
    }

    Err(ConfigError::MissingField { field: "api_key" })
}

/// Workspace configuration file structure.
#[derive(Debug, Default, serde::Deserialize)]
pub struct WorkspaceConfig {
    pub provider: Option<ProviderConfig>,
    pub analysis: Option<AnalysisConfig>,
}

/// Load workspace configuration from a TOML file.
pub fn load_workspace_config(path: &Path) -> Result<WorkspaceConfig, ConfigError> {
    let content = std::fs::read_to_string(path)?;
    toml::from_str(&content).map_err(|e| ConfigError::ParseError {
        message: e.message().to_string(),
        location: e
            .span()
            .map(|s| format!("line {}", s.start))
            .unwrap_or_else(|| "unknown".to_string()),
    })
}

/// Resolve the final provider configuration.
///
/// Resolution order:
/// 1. Explicit config (if provided)
/// 2. Workspace config file
/// 3. Auto-detection from environment
pub fn resolve_config(
    explicit: Option<ProviderConfig>,
    workspace_path: Option<&Path>,
) -> Result<ProviderConfig, ConfigError> {
    // 1. Use explicit config if provided
    if let Some(config) = explicit {
        return Ok(config);
    }

    // 2. Try workspace config
    if let Some(path) = workspace_path {
        if path.exists() {
            let workspace = load_workspace_config(path)?;
            if let Some(config) = workspace.provider {
                return Ok(config);
            }
        }
    }

    // 3. Auto-detect from environment
    auto_detect_provider().ok_or(ConfigError::NoProvider)
}

/// Resolve the analysis configuration.
///
/// Falls back to provider config if not explicitly configured.
pub fn resolve_analysis_config(
    provider_config: &ProviderConfig,
    explicit: Option<AnalysisConfig>,
    workspace_path: Option<&Path>,
) -> AnalysisConfig {
    // 1. Use explicit config if provided
    if let Some(config) = explicit {
        return config;
    }

    // 2. Try workspace config
    if let Some(path) = workspace_path {
        if path.exists() {
            if let Ok(workspace) = load_workspace_config(path) {
                if let Some(config) = workspace.analysis {
                    return config;
                }
            }
        }
    }

    // 3. Derive from provider config
    AnalysisConfig::from(provider_config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auto_detect_with_env() {
        // This test depends on environment - skip if no key set
        if env::var("OPENAI_API_KEY").is_ok() {
            let config = auto_detect_provider();
            assert!(config.is_some());
            let config = config.unwrap();
            assert_eq!(config.provider, ProviderKind::OpenAI);
        }
    }

    #[test]
    fn test_resolve_api_key_literal() {
        let config = ProviderConfig {
            api_key: Some("test-key".to_string()),
            ..Default::default()
        };
        let key = resolve_api_key(&config).unwrap();
        assert_eq!(key, "test-key");
    }
}

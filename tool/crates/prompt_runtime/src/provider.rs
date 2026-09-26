//! Provider client implementations.

use async_openai::{
    config::OpenAIConfig,
    types::{ChatCompletionRequestUserMessageArgs, CreateChatCompletionRequestArgs},
    Client as OpenAIClientLib,
};
use async_trait::async_trait;
use prompt_ir::{ProviderConfig, ProviderKind};
use reqwest::Client;
use serde_json::{json, Value};

use crate::{RuntimeError, RuntimeResult};

/// Trait for LLM provider clients.
#[async_trait]
pub trait ProviderClient: Send + Sync {
    /// Send a completion request.
    async fn complete(&self, prompt: &str) -> RuntimeResult<String>;

    /// Check if streaming is supported.
    fn supports_streaming(&self) -> bool {
        false
    }
}

/// Create a provider client from configuration.
pub fn create_client(config: &ProviderConfig) -> RuntimeResult<Box<dyn ProviderClient>> {
    match config.provider {
        ProviderKind::OpenAI => Ok(Box::new(OpenAIClient::new(config)?)),
        ProviderKind::Anthropic => Ok(Box::new(AnthropicClient::new(config)?)),
        ProviderKind::CustomHttp => Ok(Box::new(CustomHttpClient::new(config)?)),
        ProviderKind::Local => Ok(Box::new(LocalClient::new(config)?)),
        ProviderKind::OllamaCloud => Ok(Box::new(OllamaCloudClient::new(config)?)),
    }
}

/// OpenAI provider client using async-openai library.
pub struct OpenAIClient {
    client: OpenAIClientLib<OpenAIConfig>,
    model: String,
}

impl OpenAIClient {
    pub fn new(config: &ProviderConfig) -> RuntimeResult<Self> {
        let api_key = resolve_api_key(config)?;

        let openai_config = if let Some(endpoint) = &config.endpoint {
            OpenAIConfig::new()
                .with_api_key(&api_key)
                .with_api_base(endpoint)
        } else {
            OpenAIConfig::new().with_api_key(&api_key)
        };

        let client = OpenAIClientLib::with_config(openai_config);

        Ok(Self {
            client,
            model: config.model.clone(),
        })
    }
}

#[async_trait]
impl ProviderClient for OpenAIClient {
    async fn complete(&self, prompt: &str) -> RuntimeResult<String> {
        let request = CreateChatCompletionRequestArgs::default()
            .model(&self.model)
            .messages(vec![ChatCompletionRequestUserMessageArgs::default()
                .content(prompt)
                .build()
                .map_err(|e| RuntimeError::provider_error(e.to_string()))?
                .into()])
            .build()
            .map_err(|e| RuntimeError::provider_error(e.to_string()))?;

        let response = self
            .client
            .chat()
            .create(request)
            .await
            .map_err(|e| RuntimeError::connection_error(e.to_string()))?;

        response
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .ok_or_else(|| RuntimeError::provider_error("No response content"))
    }
}

/// Anthropic provider client.
pub struct AnthropicClient {
    http: Client,
    model: String,
    api_key: String,
    endpoint: String,
}

impl AnthropicClient {
    pub fn new(config: &ProviderConfig) -> RuntimeResult<Self> {
        let api_key = resolve_api_key(config)?;
        let http = Client::builder()
            .timeout(std::time::Duration::from_millis(config.timeout_ms))
            .build()
            .map_err(|e| RuntimeError::ConfigError(e.to_string()))?;

        Ok(Self {
            http,
            model: config.model.clone(),
            api_key,
            endpoint: config
                .endpoint
                .clone()
                .unwrap_or_else(|| "https://api.anthropic.com/v1/messages".to_string()),
        })
    }
}

#[async_trait]
impl ProviderClient for AnthropicClient {
    async fn complete(&self, prompt: &str) -> RuntimeResult<String> {
        let body = json!({
            "model": &self.model,
            "max_tokens": 4096,
            "messages": [{"role": "user", "content": prompt}]
        });

        let response = self
            .http
            .post(&self.endpoint)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await
            .map_err(|e| RuntimeError::connection_error(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(RuntimeError::provider_error(format!(
                "HTTP {}: {}",
                status, body
            )));
        }

        let json: Value = response
            .json()
            .await
            .map_err(|e| RuntimeError::provider_error(e.to_string()))?;

        json["content"][0]["text"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| RuntimeError::provider_error("Invalid response format"))
    }
}

/// Custom HTTP provider client (OpenAI-compatible).
pub struct CustomHttpClient {
    http: Client,
    model: String,
    api_key: String,
    endpoint: String,
}

impl CustomHttpClient {
    pub fn new(config: &ProviderConfig) -> RuntimeResult<Self> {
        let api_key = resolve_api_key(config).unwrap_or_default();
        let http = Client::builder()
            .timeout(std::time::Duration::from_millis(config.timeout_ms))
            .build()
            .map_err(|e| RuntimeError::ConfigError(e.to_string()))?;

        let endpoint = config
            .endpoint
            .clone()
            .ok_or_else(|| RuntimeError::ConfigError("CustomHttp requires endpoint".to_string()))?;

        Ok(Self {
            http,
            model: config.model.clone(),
            api_key,
            endpoint,
        })
    }
}

#[async_trait]
impl ProviderClient for CustomHttpClient {
    async fn complete(&self, prompt: &str) -> RuntimeResult<String> {
        let body = json!({
            "model": &self.model,
            "messages": [{"role": "user", "content": prompt}]
        });

        let mut request = self.http.post(&self.endpoint).json(&body);

        if !self.api_key.is_empty() {
            request = request.header("Authorization", format!("Bearer {}", self.api_key));
        }

        let response = request
            .send()
            .await
            .map_err(|e| RuntimeError::connection_error(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(RuntimeError::provider_error(format!(
                "HTTP {}: {}",
                status, body
            )));
        }

        let json: Value = response
            .json()
            .await
            .map_err(|e| RuntimeError::provider_error(e.to_string()))?;

        json["choices"][0]["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| RuntimeError::provider_error("Invalid response format"))
    }
}

/// Local provider client (Ollama, etc.).
pub struct LocalClient {
    inner: CustomHttpClient,
}

impl LocalClient {
    pub fn new(config: &ProviderConfig) -> RuntimeResult<Self> {
        let mut local_config = config.clone();
        if local_config.endpoint.is_none() {
            local_config.endpoint = Some("http://localhost:11434/v1/chat/completions".to_string());
        }
        Ok(Self {
            inner: CustomHttpClient::new(&local_config)?,
        })
    }
}

#[async_trait]
impl ProviderClient for LocalClient {
    async fn complete(&self, prompt: &str) -> RuntimeResult<String> {
        self.inner.complete(prompt).await
    }
}

/// Ollama Cloud provider client.
///
/// Ollama Cloud exposes hosted open models through an OpenAI-compatible API at
/// `https://ollama.com/v1`, authenticated with a bearer token. It is therefore a
/// thin wrapper over [`CustomHttpClient`] that fills in the cloud endpoint and the
/// `OLLAMA_API_KEY` environment variable when they are not set explicitly.
pub struct OllamaCloudClient {
    inner: CustomHttpClient,
}

/// Default OpenAI-compatible chat-completions endpoint for Ollama Cloud.
pub const OLLAMA_CLOUD_ENDPOINT: &str = "https://ollama.com/v1/chat/completions";
/// Default environment variable holding the Ollama Cloud API key.
pub const OLLAMA_CLOUD_API_KEY_ENV: &str = "OLLAMA_API_KEY";

impl OllamaCloudClient {
    pub fn new(config: &ProviderConfig) -> RuntimeResult<Self> {
        let mut cloud_config = config.clone();
        if cloud_config.endpoint.is_none() {
            cloud_config.endpoint = Some(OLLAMA_CLOUD_ENDPOINT.to_string());
        }
        if cloud_config.api_key.is_none() && cloud_config.api_key_env.is_none() {
            cloud_config.api_key_env = Some(OLLAMA_CLOUD_API_KEY_ENV.to_string());
        }
        Ok(Self {
            inner: CustomHttpClient::new(&cloud_config)?,
        })
    }
}

#[async_trait]
impl ProviderClient for OllamaCloudClient {
    async fn complete(&self, prompt: &str) -> RuntimeResult<String> {
        self.inner.complete(prompt).await
    }
}

fn resolve_api_key(config: &ProviderConfig) -> RuntimeResult<String> {
    if let Some(key) = &config.api_key {
        return Ok(key.clone());
    }

    if let Some(env_var) = &config.api_key_env {
        return std::env::var(env_var)
            .map_err(|_| RuntimeError::ConfigError(format!("API key not found: {}", env_var)));
    }

    Err(RuntimeError::ConfigError(
        "No API key configured".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn openai_config() -> ProviderConfig {
        ProviderConfig {
            provider: ProviderKind::OpenAI,
            model: "gpt-4".to_string(),
            endpoint: None,
            api_key: Some("test-key".to_string()),
            api_key_env: None,
            timeout_ms: 30000,
        }
    }

    fn anthropic_config() -> ProviderConfig {
        ProviderConfig {
            provider: ProviderKind::Anthropic,
            model: "claude-sonnet-4-20250514".to_string(),
            endpoint: None,
            api_key: Some("test-key".to_string()),
            api_key_env: None,
            timeout_ms: 60000,
        }
    }

    fn custom_http_config() -> ProviderConfig {
        ProviderConfig {
            provider: ProviderKind::CustomHttp,
            model: "custom-model".to_string(),
            endpoint: Some("http://localhost:8080/v1/chat/completions".to_string()),
            api_key: Some("test-key".to_string()),
            api_key_env: None,
            timeout_ms: 30000,
        }
    }

    fn local_config() -> ProviderConfig {
        ProviderConfig {
            provider: ProviderKind::Local,
            model: "llama3.1".to_string(),
            endpoint: None,
            api_key: None,
            api_key_env: None,
            timeout_ms: 120000,
        }
    }

    #[test]
    fn test_openai_client_creation() {
        let config = openai_config();
        let client = OpenAIClient::new(&config);
        assert!(client.is_ok());

        let client = client.unwrap();
        assert_eq!(client.model, "gpt-4");
    }

    #[test]
    fn test_anthropic_client_creation() {
        let config = anthropic_config();
        let client = AnthropicClient::new(&config);
        assert!(client.is_ok());

        let client = client.unwrap();
        assert_eq!(client.model, "claude-sonnet-4-20250514");
        assert_eq!(client.endpoint, "https://api.anthropic.com/v1/messages");
    }

    #[test]
    fn test_custom_http_client_creation() {
        let config = custom_http_config();
        let client = CustomHttpClient::new(&config);
        assert!(client.is_ok());

        let client = client.unwrap();
        assert_eq!(client.model, "custom-model");
        assert_eq!(client.endpoint, "http://localhost:8080/v1/chat/completions");
    }

    #[test]
    fn test_custom_http_requires_endpoint() {
        let mut config = custom_http_config();
        config.endpoint = None;

        let client = CustomHttpClient::new(&config);
        assert!(client.is_err());
    }

    #[test]
    fn test_local_client_default_endpoint() {
        let config = local_config();
        let client = LocalClient::new(&config);
        assert!(client.is_ok());
    }

    fn ollama_cloud_config() -> ProviderConfig {
        ProviderConfig {
            provider: ProviderKind::OllamaCloud,
            model: "gpt-oss:120b".to_string(),
            endpoint: None,
            api_key: Some("test-key".to_string()),
            api_key_env: None,
            timeout_ms: 120000,
        }
    }

    #[test]
    fn test_ollama_cloud_client_creation() {
        let config = ollama_cloud_config();
        let client = OllamaCloudClient::new(&config);
        assert!(client.is_ok());
        assert_eq!(client.unwrap().inner.endpoint, OLLAMA_CLOUD_ENDPOINT);
    }

    #[test]
    fn test_ollama_cloud_respects_explicit_endpoint() {
        let mut config = ollama_cloud_config();
        config.endpoint = Some("https://proxy.example.com/v1/chat/completions".to_string());
        let client = OllamaCloudClient::new(&config).unwrap();
        assert_eq!(
            client.inner.endpoint,
            "https://proxy.example.com/v1/chat/completions"
        );
    }

    #[test]
    fn test_create_client_ollama_cloud() {
        let config = ollama_cloud_config();
        let client = create_client(&config);
        assert!(client.is_ok());
    }

    #[test]
    fn test_create_client_openai() {
        let config = openai_config();
        let client = create_client(&config);
        assert!(client.is_ok());
    }

    #[test]
    fn test_create_client_anthropic() {
        let config = anthropic_config();
        let client = create_client(&config);
        assert!(client.is_ok());
    }

    #[test]
    fn test_create_client_custom_http() {
        let config = custom_http_config();
        let client = create_client(&config);
        assert!(client.is_ok());
    }

    #[test]
    fn test_create_client_local() {
        let config = local_config();
        let client = create_client(&config);
        assert!(client.is_ok());
    }

    #[test]
    fn test_resolve_api_key_literal() {
        let config = openai_config();
        let key = resolve_api_key(&config);
        assert!(key.is_ok());
        assert_eq!(key.unwrap(), "test-key");
    }

    #[test]
    fn test_resolve_api_key_missing() {
        let config = ProviderConfig {
            provider: ProviderKind::OpenAI,
            model: "gpt-4".to_string(),
            endpoint: None,
            api_key: None,
            api_key_env: None,
            timeout_ms: 30000,
        };

        let key = resolve_api_key(&config);
        assert!(key.is_err());
    }

    #[test]
    fn test_resolve_api_key_from_env() {
        std::env::set_var("TEST_API_KEY_12345", "env-key-value");

        let config = ProviderConfig {
            provider: ProviderKind::OpenAI,
            model: "gpt-4".to_string(),
            endpoint: None,
            api_key: None,
            api_key_env: Some("TEST_API_KEY_12345".to_string()),
            timeout_ms: 30000,
        };

        let key = resolve_api_key(&config);
        assert!(key.is_ok());
        assert_eq!(key.unwrap(), "env-key-value");

        std::env::remove_var("TEST_API_KEY_12345");
    }

    #[test]
    fn test_custom_endpoint_override() {
        let mut config = openai_config();
        config.endpoint = Some("https://custom.openai.azure.com/v1/chat/completions".to_string());

        let client = OpenAIClient::new(&config);
        assert!(client.is_ok());
    }
}

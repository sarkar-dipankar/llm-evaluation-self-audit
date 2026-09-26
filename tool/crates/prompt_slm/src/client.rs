//! Analysis client implementation.

use async_openai::{
    config::OpenAIConfig,
    types::{ChatCompletionRequestUserMessageArgs, CreateChatCompletionRequestArgs},
    Client as OpenAIClientLib,
};
use async_trait::async_trait;
use prompt_ir::{
    AnalysisConfig, GeneratedTestCase, PromptDiagnostic, PromptIR, ProviderKind,
    SuggestedBreakpoint,
};
use reqwest::Client;
use serde_json::{json, Value};
use tracing::{debug, error};

use crate::prompts;
use crate::{AnalysisError, AnalysisProvider, AnalysisResult, SuggestedFix};

/// Client for LLM-powered analysis.
pub struct AnalysisClient {
    config: AnalysisConfig,
    openai_client: Option<OpenAIClientLib<OpenAIConfig>>,
    http: Client,
}

impl AnalysisClient {
    /// Create a new analysis client.
    pub fn new(config: AnalysisConfig) -> AnalysisResult<Self> {
        let http = Client::builder()
            .timeout(std::time::Duration::from_millis(config.timeout_ms))
            .build()
            .map_err(|e| AnalysisError::ConfigError(e.to_string()))?;

        // Create OpenAI client if using OpenAI provider
        let openai_client = if config.provider == ProviderKind::OpenAI {
            let api_key = Self::resolve_api_key(&config)?;
            let openai_config = if let Some(endpoint) = &config.endpoint {
                OpenAIConfig::new()
                    .with_api_key(&api_key)
                    .with_api_base(endpoint)
            } else {
                OpenAIConfig::new().with_api_key(&api_key)
            };
            Some(OpenAIClientLib::with_config(openai_config))
        } else {
            None
        };

        Ok(Self {
            config,
            openai_client,
            http,
        })
    }

    /// Resolve the API key from config or environment.
    fn resolve_api_key(config: &AnalysisConfig) -> AnalysisResult<String> {
        if let Some(key) = &config.api_key {
            return Ok(key.clone());
        }

        if let Some(env_var) = &config.api_key_env {
            return std::env::var(env_var).map_err(|_| {
                AnalysisError::ConfigError(format!("API key not found: {}", env_var))
            });
        }

        Err(AnalysisError::ConfigError(
            "No API key configured".to_string(),
        ))
    }

    /// Get the endpoint URL for non-OpenAI providers.
    fn endpoint(&self) -> String {
        if let Some(endpoint) = &self.config.endpoint {
            return endpoint.clone();
        }

        match self.config.provider {
            ProviderKind::OpenAI => "https://api.openai.com/v1/chat/completions".to_string(),
            ProviderKind::Anthropic => "https://api.anthropic.com/v1/messages".to_string(),
            ProviderKind::CustomHttp => {
                panic!("CustomHttp requires explicit endpoint")
            }
            ProviderKind::Local => "http://localhost:11434/v1/chat/completions".to_string(),
            ProviderKind::OllamaCloud => "https://ollama.com/v1/chat/completions".to_string(),
        }
    }

    /// Make a completion request to the LLM.
    async fn complete(&self, prompt: &str) -> AnalysisResult<String> {
        // Use async-openai for OpenAI provider
        if let Some(client) = &self.openai_client {
            return self.complete_openai(client, prompt).await;
        }

        // Fall back to raw HTTP for other providers
        self.complete_http(prompt).await
    }

    /// Complete using async-openai library.
    async fn complete_openai(
        &self,
        client: &OpenAIClientLib<OpenAIConfig>,
        prompt: &str,
    ) -> AnalysisResult<String> {
        debug!("Making OpenAI request with model {}", self.config.model);

        let request = CreateChatCompletionRequestArgs::default()
            .model(&self.config.model)
            .temperature(self.config.temperature)
            .messages(vec![ChatCompletionRequestUserMessageArgs::default()
                .content(prompt)
                .build()
                .map_err(|e| AnalysisError::ConfigError(e.to_string()))?
                .into()])
            .build()
            .map_err(|e| AnalysisError::ConfigError(e.to_string()))?;

        let response = client
            .chat()
            .create(request)
            .await
            .map_err(|e| AnalysisError::ConnectionError(e.to_string()))?;

        response
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .ok_or_else(|| AnalysisError::ProviderError("No response content".to_string()))
    }

    /// Complete using raw HTTP (for Anthropic, CustomHttp, Local).
    async fn complete_http(&self, prompt: &str) -> AnalysisResult<String> {
        let api_key = Self::resolve_api_key(&self.config)?;
        let endpoint = self.endpoint();

        debug!("Making HTTP request to {}", endpoint);

        let request_body = match self.config.provider {
            ProviderKind::Anthropic => json!({
                "model": &self.config.model,
                "max_tokens": 4096,
                "temperature": self.config.temperature,
                "messages": [{"role": "user", "content": prompt}]
            }),
            _ => json!({
                "model": &self.config.model,
                "temperature": self.config.temperature,
                "messages": [{"role": "user", "content": prompt}]
            }),
        };

        let mut request = self.http.post(&endpoint).json(&request_body);

        // Set auth headers based on provider
        request = match self.config.provider {
            ProviderKind::Anthropic => request
                .header("x-api-key", &api_key)
                .header("anthropic-version", "2023-06-01"),
            _ => request.header("Authorization", format!("Bearer {}", api_key)),
        };

        let response = request
            .send()
            .await
            .map_err(|e| AnalysisError::ConnectionError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            error!("Analysis request failed: {} - {}", status, body);
            return Err(AnalysisError::ProviderError(format!(
                "HTTP {}: {}",
                status, body
            )));
        }

        let response_json: Value = response
            .json()
            .await
            .map_err(|e| AnalysisError::ParseError(e.to_string()))?;

        // Extract content based on provider response format
        let content = match self.config.provider {
            ProviderKind::Anthropic => response_json["content"][0]["text"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            _ => response_json["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or("")
                .to_string(),
        };

        Ok(content)
    }

    /// Parse JSON from LLM response, handling markdown code blocks.
    fn parse_json_response<T: serde::de::DeserializeOwned>(
        &self,
        response: &str,
    ) -> AnalysisResult<T> {
        // Strip markdown code blocks if present
        let trimmed = response.trim();
        let json_str = if trimmed.starts_with("```json") {
            trimmed
                .strip_prefix("```json")
                .unwrap()
                .strip_suffix("```")
                .unwrap_or(trimmed)
                .trim()
        } else if trimmed.starts_with("```") {
            trimmed
                .strip_prefix("```")
                .unwrap()
                .strip_suffix("```")
                .unwrap_or(trimmed)
                .trim()
        } else {
            trimmed
        };

        serde_json::from_str(json_str).map_err(|e| {
            AnalysisError::ParseError(format!("Failed to parse JSON: {} - {}", e, json_str))
        })
    }
}

#[async_trait]
impl AnalysisProvider for AnalysisClient {
    async fn generate_ir(
        &self,
        content: &str,
        uri: &str,
        version: i32,
    ) -> AnalysisResult<PromptIR> {
        let prompt = prompts::ir_generation_prompt(content);
        let response = self.complete(&prompt).await?;

        #[derive(serde::Deserialize)]
        struct IrResponse {
            nodes: Vec<prompt_ir::IRNode>,
            meta: Option<prompt_ir::IRMeta>,
        }

        let parsed: IrResponse = self.parse_json_response(&response)?;

        Ok(PromptIR {
            uri: uri.to_string(),
            version,
            nodes: parsed.nodes,
            meta: parsed.meta,
        })
    }

    async fn lint(&self, content: &str, ir: &PromptIR) -> AnalysisResult<Vec<PromptDiagnostic>> {
        let ir_json =
            serde_json::to_string(ir).map_err(|e| AnalysisError::ParseError(e.to_string()))?;
        let prompt = prompts::lint_prompt(content, &ir_json);
        let response = self.complete(&prompt).await?;

        #[derive(serde::Deserialize)]
        struct LintResponse {
            diagnostics: Vec<PromptDiagnostic>,
        }

        let parsed: LintResponse = self.parse_json_response(&response)?;
        Ok(parsed.diagnostics)
    }

    async fn suggest_breakpoints(
        &self,
        content: &str,
        ir: &PromptIR,
    ) -> AnalysisResult<Vec<SuggestedBreakpoint>> {
        let ir_json =
            serde_json::to_string(ir).map_err(|e| AnalysisError::ParseError(e.to_string()))?;
        let prompt = prompts::suggest_breakpoints_prompt(content, &ir_json);
        let response = self.complete(&prompt).await?;

        #[derive(serde::Deserialize)]
        struct BreakpointResponse {
            suggestions: Vec<SuggestedBreakpoint>,
        }

        let parsed: BreakpointResponse = self.parse_json_response(&response)?;
        Ok(parsed.suggestions)
    }

    async fn generate_tests(
        &self,
        content: &str,
        ir: &PromptIR,
        target_node_id: &str,
    ) -> AnalysisResult<Vec<GeneratedTestCase>> {
        let ir_json =
            serde_json::to_string(ir).map_err(|e| AnalysisError::ParseError(e.to_string()))?;

        let target_node = ir.find_node(target_node_id).ok_or_else(|| {
            AnalysisError::ConfigError(format!("Node not found: {}", target_node_id))
        })?;

        let node_json = serde_json::to_string(target_node)
            .map_err(|e| AnalysisError::ParseError(e.to_string()))?;

        let prompt = prompts::generate_tests_prompt(content, &node_json, &ir_json);
        let response = self.complete(&prompt).await?;

        #[derive(serde::Deserialize)]
        struct TestResponse {
            tests: Vec<GeneratedTestCase>,
        }

        let parsed: TestResponse = self.parse_json_response(&response)?;
        Ok(parsed.tests)
    }

    async fn suggest_fixes(
        &self,
        content: &str,
        diagnostics: &[PromptDiagnostic],
    ) -> AnalysisResult<Vec<SuggestedFix>> {
        if diagnostics.is_empty() {
            return Ok(Vec::new());
        }

        let diagnostics_json = serde_json::to_string(diagnostics)
            .map_err(|e| AnalysisError::ParseError(e.to_string()))?;

        let prompt = prompts::suggest_fixes_prompt(content, &diagnostics_json);
        let response = self.complete(&prompt).await?;

        #[derive(serde::Deserialize)]
        struct FixResponse {
            fixes: Vec<SuggestedFix>,
        }

        let parsed: FixResponse = self.parse_json_response(&response)?;
        Ok(parsed.fixes)
    }

    async fn generate_skill_ir(
        &self,
        content: &str,
        frontmatter_json: &str,
        uri: &str,
        version: i32,
    ) -> AnalysisResult<PromptIR> {
        let prompt = prompts::skill_ir_generation_prompt(content, frontmatter_json);
        let response = self.complete(&prompt).await?;

        #[derive(serde::Deserialize)]
        struct IrResponse {
            nodes: Vec<prompt_ir::IRNode>,
            meta: Option<prompt_ir::IRMeta>,
        }

        let parsed: IrResponse = self.parse_json_response(&response)?;

        Ok(PromptIR {
            uri: uri.to_string(),
            version,
            nodes: parsed.nodes,
            meta: parsed.meta,
        })
    }

    async fn lint_skill(
        &self,
        content: &str,
        frontmatter_json: &str,
        ir: &PromptIR,
    ) -> AnalysisResult<Vec<PromptDiagnostic>> {
        let ir_json =
            serde_json::to_string(ir).map_err(|e| AnalysisError::ParseError(e.to_string()))?;
        let prompt = prompts::skill_lint_prompt(content, &ir_json, frontmatter_json);
        let response = self.complete(&prompt).await?;

        #[derive(serde::Deserialize)]
        struct LintResponse {
            diagnostics: Vec<PromptDiagnostic>,
        }

        let parsed: LintResponse = self.parse_json_response(&response)?;
        Ok(parsed.diagnostics)
    }
}

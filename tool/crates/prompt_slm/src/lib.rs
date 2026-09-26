//! LLM-powered analysis for PromptDbg.
//!
//! This crate provides:
//! - IR generation from prompt files
//! - Linting and diagnostics
//! - Breakpoint suggestions
//! - Test case generation
//!
//! The "SLM" name refers to the analysis role, not necessarily a small model.
//! Any LLM provider can be used with specialized prompts.

use async_trait::async_trait;
use prompt_ir::{GeneratedTestCase, PromptDiagnostic, PromptIR, SuggestedBreakpoint};
use thiserror::Error;

pub mod client;
pub mod prompts;

pub use client::AnalysisClient;

#[derive(Error, Debug)]
pub enum AnalysisError {
    #[error("Failed to connect to analysis provider: {0}")]
    ConnectionError(String),

    #[error("Failed to parse analysis response: {0}")]
    ParseError(String),

    #[error("Analysis request timed out")]
    Timeout,

    #[error("Analysis provider error: {0}")]
    ProviderError(String),

    #[error("Invalid configuration: {0}")]
    ConfigError(String),
}

/// Result type for analysis operations.
pub type AnalysisResult<T> = Result<T, AnalysisError>;

/// A suggested fix for a diagnostic.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SuggestedFix {
    /// Description of the fix
    pub title: String,
    /// The diagnostic this fixes
    pub diagnostic_code: String,
    /// Text edits to apply
    pub edits: Vec<TextEdit>,
    /// Whether this fix is preferred (should be auto-applied)
    pub is_preferred: bool,
}

/// A text edit operation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TextEdit {
    /// Start line (0-indexed)
    pub start_line: u32,
    /// Start column (0-indexed)
    pub start_col: u32,
    /// End line (0-indexed)
    pub end_line: u32,
    /// End column (0-indexed)
    pub end_col: u32,
    /// New text to insert
    pub new_text: String,
}

/// Trait for analysis providers.
#[async_trait]
pub trait AnalysisProvider: Send + Sync {
    /// Generate IR from prompt content.
    async fn generate_ir(&self, content: &str, uri: &str, version: i32)
        -> AnalysisResult<PromptIR>;

    /// Lint prompt content and return diagnostics.
    async fn lint(&self, content: &str, ir: &PromptIR) -> AnalysisResult<Vec<PromptDiagnostic>>;

    /// Suggest breakpoint locations.
    async fn suggest_breakpoints(
        &self,
        content: &str,
        ir: &PromptIR,
    ) -> AnalysisResult<Vec<SuggestedBreakpoint>>;

    /// Generate test cases for a specific node.
    async fn generate_tests(
        &self,
        content: &str,
        ir: &PromptIR,
        target_node_id: &str,
    ) -> AnalysisResult<Vec<GeneratedTestCase>>;

    /// Generate suggested fixes for diagnostics.
    async fn suggest_fixes(
        &self,
        content: &str,
        diagnostics: &[PromptDiagnostic],
    ) -> AnalysisResult<Vec<SuggestedFix>>;

    /// Generate IR from a Claude Code skill file.
    async fn generate_skill_ir(
        &self,
        content: &str,
        frontmatter_json: &str,
        uri: &str,
        version: i32,
    ) -> AnalysisResult<PromptIR>;

    /// Lint a Claude Code skill file.
    async fn lint_skill(
        &self,
        content: &str,
        frontmatter_json: &str,
        ir: &PromptIR,
    ) -> AnalysisResult<Vec<PromptDiagnostic>>;
}

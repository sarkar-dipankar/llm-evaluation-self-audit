//! Diagnostic, test case, and breakpoint suggestion types.

use serde::{Deserialize, Serialize};

use crate::TextRange;

/// Severity level for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
}

impl DiagnosticSeverity {
    /// Convert to LSP severity (1=Error, 2=Warning, 3=Info, 4=Hint).
    pub fn to_lsp(&self) -> i32 {
        match self {
            DiagnosticSeverity::Error => 1,
            DiagnosticSeverity::Warning => 2,
            DiagnosticSeverity::Info => 3,
        }
    }
}

/// A diagnostic message from the template parser or SLM linter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptDiagnostic {
    pub message: String,
    pub range: TextRange,
    pub severity: DiagnosticSeverity,
    /// Optional error code (e.g., "prompt/conflict-rule")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

impl PromptDiagnostic {
    pub fn error(message: impl Into<String>, range: TextRange) -> Self {
        Self {
            message: message.into(),
            range,
            severity: DiagnosticSeverity::Error,
            code: None,
        }
    }

    pub fn warning(message: impl Into<String>, range: TextRange) -> Self {
        Self {
            message: message.into(),
            range,
            severity: DiagnosticSeverity::Warning,
            code: None,
        }
    }

    pub fn info(message: impl Into<String>, range: TextRange) -> Self {
        Self {
            message: message.into(),
            range,
            severity: DiagnosticSeverity::Info,
            code: None,
        }
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }
}

/// A suggested breakpoint location.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestedBreakpoint {
    /// ID of the node to break on
    pub node_id: String,
    /// Reason for the suggestion
    pub reason: String,
    /// Confidence score (0.0-1.0)
    pub confidence: f32,
}

/// Type of generated test case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TestKind {
    Positive,
    Negative,
    Edge,
}

impl std::fmt::Display for TestKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TestKind::Positive => write!(f, "Positive"),
            TestKind::Negative => write!(f, "Negative"),
            TestKind::Edge => write!(f, "Edge"),
        }
    }
}

/// A generated test case for a rule or condition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedTestCase {
    /// Unique test ID
    pub id: String,
    /// ID of the target rule/condition
    pub target_node_id: String,
    /// Type of test
    pub kind: TestKind,
    /// User input to test with
    pub input: String,
    /// Expected behavior description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_behavior: Option<String>,
    /// Why this test case matters
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diagnostic_builders() {
        let range = TextRange::new(1, 0, 1, 10);

        let error = PromptDiagnostic::error("Test error", range).with_code("test/error");
        assert_eq!(error.severity, DiagnosticSeverity::Error);
        assert_eq!(error.code, Some("test/error".to_string()));

        let warning = PromptDiagnostic::warning("Test warning", range);
        assert_eq!(warning.severity, DiagnosticSeverity::Warning);
    }

    #[test]
    fn test_severity_to_lsp() {
        assert_eq!(DiagnosticSeverity::Error.to_lsp(), 1);
        assert_eq!(DiagnosticSeverity::Warning.to_lsp(), 2);
        assert_eq!(DiagnosticSeverity::Info.to_lsp(), 3);
    }
}

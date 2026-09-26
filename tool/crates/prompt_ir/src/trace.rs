//! Runtime execution trace types.

use serde::{Deserialize, Serialize};

/// Kind of trace step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TraceStepKind {
    Plan,
    Execution,
    Breakpoint,
    Summary,
}

impl std::fmt::Display for TraceStepKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TraceStepKind::Plan => write!(f, "Plan"),
            TraceStepKind::Execution => write!(f, "Execution"),
            TraceStepKind::Breakpoint => write!(f, "Breakpoint"),
            TraceStepKind::Summary => write!(f, "Summary"),
        }
    }
}

/// A complete execution trace for a prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptTrace {
    /// Unique session identifier
    pub session_id: String,
    /// Ordered trace steps
    pub steps: Vec<TraceStep>,
    /// Whether execution completed successfully
    pub completed: bool,
    /// Final answer if completed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_answer: Option<String>,
}

impl PromptTrace {
    /// Create a new trace for a session.
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            steps: Vec::new(),
            completed: false,
            final_answer: None,
        }
    }

    /// Add a step to the trace.
    pub fn add_step(&mut self, step: TraceStep) {
        self.steps.push(step);
    }

    /// Mark the trace as completed with an answer.
    pub fn complete(&mut self, answer: impl Into<String>) {
        self.completed = true;
        self.final_answer = Some(answer.into());
    }

    /// Get all rules that were used during execution.
    pub fn rules_used(&self) -> Vec<&str> {
        self.steps
            .iter()
            .flat_map(|s| s.rules_used.iter().map(|r| r.as_str()))
            .collect()
    }

    /// Get all breakpoints that were hit.
    pub fn breakpoints_hit(&self) -> Vec<&str> {
        self.steps
            .iter()
            .flat_map(|s| s.breakpoints_hit.iter().map(|b| b.as_str()))
            .collect()
    }
}

/// A single step in the execution trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceStep {
    /// Step index (0-based)
    pub index: u32,
    /// Type of step
    pub kind: TraceStepKind,
    /// ISO 8601 timestamp
    pub timestamp: String,
    /// Rules that were applied in this step
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules_used: Vec<String>,
    /// Rules that were considered but not applied
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules_considered: Vec<String>,
    /// Breakpoints that were hit
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub breakpoints_hit: Vec<String>,
    /// Human-readable explanation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explanation: Option<String>,
    /// Partial answer at this step
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partial_answer: Option<String>,
}

impl TraceStep {
    /// Create a new trace step.
    pub fn new(index: u32, kind: TraceStepKind) -> Self {
        Self {
            index,
            kind,
            timestamp: chrono_lite_timestamp(),
            rules_used: Vec::new(),
            rules_considered: Vec::new(),
            breakpoints_hit: Vec::new(),
            explanation: None,
            partial_answer: None,
        }
    }

    pub fn with_explanation(mut self, explanation: impl Into<String>) -> Self {
        self.explanation = Some(explanation.into());
        self
    }

    pub fn with_partial_answer(mut self, answer: impl Into<String>) -> Self {
        self.partial_answer = Some(answer.into());
        self
    }

    pub fn with_rules_used(mut self, rules: Vec<String>) -> Self {
        self.rules_used = rules;
        self
    }

    pub fn with_breakpoint_hit(mut self, breakpoint: impl Into<String>) -> Self {
        self.breakpoints_hit.push(breakpoint.into());
        self
    }
}

/// Generate a simple ISO 8601 timestamp without external dependencies.
fn chrono_lite_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();

    // Simple conversion - not handling leap seconds, etc.
    let days_since_epoch = secs / 86400;
    let time_of_day = secs % 86400;

    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Approximate date calculation (doesn't account for leap years perfectly)
    let mut year = 1970;
    let mut remaining_days = days_since_epoch;

    loop {
        let days_in_year = if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
            366
        } else {
            365
        };
        if remaining_days < days_in_year {
            break;
        }
        remaining_days -= days_in_year;
        year += 1;
    }

    let is_leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_months: [u64; 12] = if is_leap {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 1;
    for days in days_in_months {
        if remaining_days < days {
            break;
        }
        remaining_days -= days;
        month += 1;
    }
    let day = remaining_days + 1;

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hours, minutes, seconds
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trace_creation() {
        let mut trace = PromptTrace::new("test-session");
        assert!(!trace.completed);
        assert!(trace.steps.is_empty());

        trace.add_step(TraceStep::new(0, TraceStepKind::Plan));
        assert_eq!(trace.steps.len(), 1);

        trace.complete("Hello, world!");
        assert!(trace.completed);
        assert_eq!(trace.final_answer, Some("Hello, world!".to_string()));
    }

    #[test]
    fn test_trace_step_builder() {
        let step = TraceStep::new(0, TraceStepKind::Execution)
            .with_explanation("Applying greeting rule")
            .with_rules_used(vec!["RULE:GREETING".to_string()])
            .with_partial_answer("Hello");

        assert_eq!(step.kind, TraceStepKind::Execution);
        assert_eq!(step.explanation, Some("Applying greeting rule".to_string()));
        assert_eq!(step.rules_used, vec!["RULE:GREETING"]);
    }

    #[test]
    fn test_timestamp_format() {
        let ts = chrono_lite_timestamp();
        // Should be in ISO 8601 format
        assert!(ts.contains('T'));
        assert!(ts.ends_with('Z'));
    }
}

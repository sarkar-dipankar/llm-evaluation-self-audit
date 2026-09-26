//! DAP protocol types.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Base protocol message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolMessage {
    pub seq: i64,
    #[serde(rename = "type")]
    pub msg_type: String,
}

/// A request message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub seq: i64,
    #[serde(rename = "type")]
    pub msg_type: String,
    pub command: String,
    #[serde(default)]
    pub arguments: Option<Value>,
}

/// A response message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub seq: i64,
    #[serde(rename = "type")]
    pub msg_type: String,
    pub request_seq: i64,
    pub success: bool,
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
}

impl Response {
    pub fn success(request: &Request, body: Option<Value>) -> Self {
        Self {
            seq: super::next_seq(),
            msg_type: "response".to_string(),
            request_seq: request.seq,
            success: true,
            command: request.command.clone(),
            message: None,
            body,
        }
    }

    pub fn error(request: &Request, message: &str) -> Self {
        Self {
            seq: super::next_seq(),
            msg_type: "response".to_string(),
            request_seq: request.seq,
            success: false,
            command: request.command.clone(),
            message: Some(message.to_string()),
            body: None,
        }
    }
}

/// An event message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub seq: i64,
    #[serde(rename = "type")]
    pub msg_type: String,
    pub event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
}

impl Event {
    pub fn new(event: &str, body: Option<Value>) -> Self {
        Self {
            seq: super::next_seq(),
            msg_type: "event".to_string(),
            event: event.to_string(),
            body,
        }
    }

    pub fn initialized() -> Self {
        Self::new("initialized", None)
    }

    pub fn stopped(reason: &str, thread_id: i64, description: Option<&str>) -> Self {
        let mut body = serde_json::json!({
            "reason": reason,
            "threadId": thread_id,
        });

        if let Some(desc) = description {
            body["description"] = serde_json::json!(desc);
        }

        Self::new("stopped", Some(body))
    }

    pub fn terminated() -> Self {
        Self::new("terminated", None)
    }

    pub fn exited(exit_code: i32) -> Self {
        Self::new("exited", Some(serde_json::json!({ "exitCode": exit_code })))
    }

    pub fn output(category: &str, output: &str) -> Self {
        Self::new(
            "output",
            Some(serde_json::json!({
                "category": category,
                "output": output,
            })),
        )
    }

    pub fn breakpoint(reason: &str, breakpoint: &super::Breakpoint) -> Self {
        Self::new(
            "breakpoint",
            Some(serde_json::json!({
                "reason": reason,
                "breakpoint": breakpoint,
            })),
        )
    }
}

/// Launch request arguments.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchArguments {
    /// Path to the prompt file
    pub program: String,
    /// Working directory
    pub cwd: Option<String>,
    /// Context variables (JSON)
    pub context: Option<Value>,
    /// Context file path
    pub context_file: Option<String>,
    /// Template-only mode (no LLM call)
    #[serde(default)]
    pub template_only: bool,
    /// Stop on entry
    #[serde(default)]
    pub stop_on_entry: bool,

    // Skill-specific arguments
    /// Arguments for skill $ARGUMENTS substitution
    #[serde(default)]
    pub skill_args: Option<String>,
    /// Session ID (auto-generated if not provided)
    #[serde(default)]
    pub session_id: Option<String>,
    /// Disable command execution in skills
    #[serde(default)]
    pub no_commands: bool,
}

/// Set breakpoints request arguments.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetBreakpointsArguments {
    pub source: super::Source,
    #[serde(default)]
    pub breakpoints: Vec<SourceBreakpoint>,
}

/// A source breakpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceBreakpoint {
    pub line: u32,
    pub condition: Option<String>,
}

/// Stack trace request arguments.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackTraceArguments {
    pub thread_id: i64,
    pub start_frame: Option<i64>,
    pub levels: Option<i64>,
}

/// Scopes request arguments.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopesArguments {
    pub frame_id: i64,
}

/// Variables request arguments.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VariablesArguments {
    pub variables_reference: i64,
}

/// Evaluate request arguments.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluateArguments {
    pub expression: String,
    pub frame_id: Option<i64>,
    pub context: Option<String>,
}

/// Initialize request arguments.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeArguments {
    pub client_id: Option<String>,
    pub client_name: Option<String>,
    pub adapter_id: String,
    #[serde(default)]
    pub lines_start_at1: bool,
    #[serde(default)]
    pub columns_start_at1: bool,
    pub path_format: Option<String>,
    #[serde(default)]
    pub supports_variable_type: bool,
    #[serde(default)]
    pub supports_variable_paging: bool,
    #[serde(default)]
    pub supports_run_in_terminal_request: bool,
}

/// Capabilities sent in initialize response.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub supports_configuration_done_request: bool,
    pub supports_function_breakpoints: bool,
    pub supports_conditional_breakpoints: bool,
    pub supports_evaluate_for_hovers: bool,
    pub supports_step_back: bool,
    pub supports_set_variable: bool,
    pub supports_restart_frame: bool,
    pub supports_goto_targets_request: bool,
    pub supports_step_in_targets_request: bool,
    pub supports_completions_request: bool,
    pub supports_modules_request: bool,
    pub supports_restart_request: bool,
    pub supports_value_formatting_options: bool,
    pub supports_exception_info_request: bool,
    pub supports_terminate_request: bool,
    pub supports_delayed_stack_trace_loading: bool,
    pub supports_loaded_sources_request: bool,
    pub supports_log_points: bool,
    pub supports_terminate_threads_request: bool,
    pub supports_set_expression: bool,
    pub supports_terminate_debuggee: bool,
    pub supports_read_memory_request: bool,
    pub supports_disassemble_request: bool,
    pub supports_cancel_request: bool,
    pub supports_breakpoint_locations_request: bool,
    pub supports_clipboard_context: bool,
    pub supports_stepping_granularity: bool,
    pub supports_instruction_breakpoints: bool,
    pub supports_exception_filter_options: bool,
}

impl Capabilities {
    pub fn promptdbg() -> Self {
        Self {
            supports_configuration_done_request: true,
            supports_evaluate_for_hovers: true,
            supports_terminate_request: true,
            ..Default::default()
        }
    }
}

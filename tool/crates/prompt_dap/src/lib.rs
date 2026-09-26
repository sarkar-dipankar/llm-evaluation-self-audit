//! Debug Adapter Protocol implementation for PromptDbg.
//!
//! Provides:
//! - Launch/attach with proper runtime integration
//! - Breakpoints with conditional support
//! - Step/continue execution control
//! - Variable inspection from template context and render results

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};

use prompt_ir::{PromptIR, TraceStep};
use prompt_runtime::{BreakpointLocation, CommandBreakpoint, SkillExecutionResult};
use prompt_template::skill_directory::SkillDirectory;
use prompt_template::RenderResult;
use serde::{Deserialize, Serialize};
use serde_json::Value;

mod protocol;
mod server;

pub use protocol::*;
pub use server::DebugServer;

/// Sequence number generator for DAP messages.
static SEQ: AtomicI64 = AtomicI64::new(1);

fn next_seq() -> i64 {
    SEQ.fetch_add(1, Ordering::SeqCst)
}

/// Debug session state.
#[derive(Debug)]
pub struct DebugSession {
    /// Source file being debugged
    pub source_path: Option<PathBuf>,
    /// Source content
    pub source_content: Option<String>,
    /// Generated IR
    pub ir: Option<PromptIR>,
    /// Template context (variables)
    pub context: Value,
    /// Set breakpoints with conditions
    pub breakpoints: Vec<BreakpointLocation>,
    /// Current execution state
    pub state: ExecutionState,
    /// Execution trace
    pub trace: Vec<TraceStep>,
    /// Current step index
    pub current_step: usize,
    /// Variables at current step
    pub variables: HashMap<String, Value>,
    /// Template-only mode
    pub template_only: bool,
    /// Source lines for mapping
    pub source_lines: Vec<String>,
    /// Template render result
    pub render_result: Option<RenderResult>,
    /// Stop on entry
    pub stop_on_entry: bool,

    // Skill debugging fields
    /// Whether this is a skill debug session
    pub is_skill: bool,
    /// Skill directory (for multi-file skills)
    pub skill_dir: Option<SkillDirectory>,
    /// Skill execution result
    pub skill_result: Option<SkillExecutionResult>,
    /// Current skill execution phase
    pub skill_phase: SkillDebugPhase,
    /// Command breakpoints (by line or pattern)
    pub command_breakpoints: Vec<CommandBreakpoint>,
}

/// Execution state of the debug session.
#[derive(Debug, Clone, PartialEq)]
pub enum ExecutionState {
    /// Not started
    NotStarted,
    /// Running
    Running,
    /// Paused at breakpoint
    Paused { line: u32, reason: String },
    /// Stopped (completed or error)
    Stopped { reason: String },
}

/// Skill execution phase for debugging.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum SkillDebugPhase {
    #[default]
    NotStarted,
    Substitution,
    ReferenceLoading,
    CommandExecution {
        command_index: usize,
    },
    PromptBuild,
    LlmCall,
    Completed,
}

impl Default for DebugSession {
    fn default() -> Self {
        Self {
            source_path: None,
            source_content: None,
            ir: None,
            context: Value::Object(Default::default()),
            breakpoints: Vec::new(),
            state: ExecutionState::NotStarted,
            trace: Vec::new(),
            current_step: 0,
            variables: HashMap::new(),
            template_only: false,
            source_lines: Vec::new(),
            render_result: None,
            stop_on_entry: false,
            // Skill debugging fields
            is_skill: false,
            skill_dir: None,
            skill_result: None,
            skill_phase: SkillDebugPhase::default(),
            command_breakpoints: Vec::new(),
        }
    }
}

impl DebugSession {
    /// Check if a line has a breakpoint.
    pub fn has_breakpoint(&self, line: u32) -> bool {
        self.breakpoints.iter().any(|bp| bp.line == line)
    }

    /// Check if a line has a breakpoint with its condition evaluated.
    pub fn check_breakpoint(&self, line: u32) -> bool {
        for bp in &self.breakpoints {
            if bp.line == line {
                // If there's a condition, evaluate it
                if let Some(condition) = &bp.condition {
                    return self.evaluate_condition(condition);
                }
                return true;
            }
        }
        false
    }

    /// Evaluate a condition expression against the context.
    fn evaluate_condition(&self, condition: &str) -> bool {
        if let Some(value) = self.get_value_by_path(condition) {
            match value {
                Value::Bool(b) => *b,
                Value::Null => false,
                Value::String(s) => !s.is_empty(),
                Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
                Value::Array(a) => !a.is_empty(),
                Value::Object(o) => !o.is_empty(),
            }
        } else {
            false
        }
    }

    /// Get value by dot-separated path from context.
    fn get_value_by_path(&self, path: &str) -> Option<&Value> {
        let parts: Vec<&str> = path.split('.').collect();
        let mut current = &self.context;

        for part in parts {
            match current {
                Value::Object(map) => {
                    current = map.get(part)?;
                }
                _ => return None,
            }
        }

        Some(current)
    }

    /// Get the current line number based on step index.
    pub fn current_line(&self) -> u32 {
        // Map step index to line number (1-based)
        // In a real implementation, this would use IR node ranges
        (self.current_step as u32)
            .saturating_add(1)
            .min(self.source_lines.len() as u32)
    }

    /// Get variables for the current scope.
    pub fn get_variables(&self) -> Vec<Variable> {
        let mut vars = Vec::new();

        // Add context variables
        if let Value::Object(map) = &self.context {
            for (key, value) in map {
                vars.push(Variable {
                    name: key.clone(),
                    value: format_value(value),
                    type_name: Some(value_type(value)),
                    variables_reference: if matches!(value, Value::Object(_) | Value::Array(_)) {
                        1
                    } else {
                        0
                    },
                });
            }
        }

        // Add render result info if available
        if let Some(render) = &self.render_result {
            vars.push(Variable {
                name: "_rendered_output".to_string(),
                value: if render.output.len() > 100 {
                    format!("\"{}...\"", &render.output[..100])
                } else {
                    format!("\"{}\"", render.output)
                },
                type_name: Some("string".to_string()),
                variables_reference: 0,
            });

            vars.push(Variable {
                name: "_variables_used".to_string(),
                value: format!("{:?}", render.variables_used),
                type_name: Some("array".to_string()),
                variables_reference: 0,
            });

            vars.push(Variable {
                name: "_branches_count".to_string(),
                value: render.branches_taken.len().to_string(),
                type_name: Some("number".to_string()),
                variables_reference: 0,
            });

            vars.push(Variable {
                name: "_loops_count".to_string(),
                value: render.loop_iterations.len().to_string(),
                type_name: Some("number".to_string()),
                variables_reference: 0,
            });
        }

        // Add trace step info if available
        if let Some(step) = self.trace.get(self.current_step) {
            vars.push(Variable {
                name: "_step_kind".to_string(),
                value: format!("{}", step.kind),
                type_name: Some("string".to_string()),
                variables_reference: 0,
            });

            if let Some(explanation) = &step.explanation {
                vars.push(Variable {
                    name: "_explanation".to_string(),
                    value: format!("\"{}\"", explanation),
                    type_name: Some("string".to_string()),
                    variables_reference: 0,
                });
            }

            if let Some(answer) = &step.partial_answer {
                vars.push(Variable {
                    name: "_partial_answer".to_string(),
                    value: if answer.len() > 200 {
                        format!("\"{}...\"", &answer[..200])
                    } else {
                        format!("\"{}\"", answer)
                    },
                    type_name: Some("string".to_string()),
                    variables_reference: 0,
                });
            }

            if !step.rules_used.is_empty() {
                vars.push(Variable {
                    name: "_rules_used".to_string(),
                    value: format!("{:?}", step.rules_used),
                    type_name: Some("array".to_string()),
                    variables_reference: 0,
                });
            }
        }

        vars
    }

    /// Get variables for a specific scope (context or trace).
    pub fn get_variables_for_scope(&self, scope_id: i64) -> Vec<Variable> {
        match scope_id {
            1 => {
                // Context scope - just context variables
                let mut vars = Vec::new();
                if let Value::Object(map) = &self.context {
                    for (key, value) in map {
                        vars.push(Variable {
                            name: key.clone(),
                            value: format_value(value),
                            type_name: Some(value_type(value)),
                            variables_reference: 0,
                        });
                    }
                }
                vars
            }
            2 => {
                // Trace/debug scope
                let mut vars = Vec::new();

                if let Some(render) = &self.render_result {
                    // Show branch info
                    for (i, branch) in render.branches_taken.iter().enumerate() {
                        vars.push(Variable {
                            name: format!("branch_{}", i),
                            value: format!(
                                "{}: {} ({})",
                                branch.condition,
                                branch.branch_executed,
                                if branch.taken { "taken" } else { "not taken" }
                            ),
                            type_name: Some("branch".to_string()),
                            variables_reference: 0,
                        });
                    }

                    // Show loop info
                    for (i, loop_info) in render.loop_iterations.iter().enumerate() {
                        vars.push(Variable {
                            name: format!("loop_{}", i),
                            value: format!(
                                "for {} in {}: {} iterations",
                                loop_info.variable, loop_info.collection, loop_info.iteration_count
                            ),
                            type_name: Some("loop".to_string()),
                            variables_reference: 0,
                        });
                    }

                    // Show variable accesses
                    for access in &render.variable_accesses {
                        vars.push(Variable {
                            name: format!("access:{}", access.name),
                            value: format!("{} ({})", access.value, access.value_type),
                            type_name: Some("access".to_string()),
                            variables_reference: 0,
                        });
                    }
                }

                vars
            }
            // Skill-specific scopes
            3 if self.is_skill => self.get_skill_variables(),
            4 if self.is_skill => self.get_command_variables(),
            5 if self.is_skill => self.get_reference_variables(),
            _ => Vec::new(),
        }
    }

    /// Get skill metadata variables.
    fn get_skill_variables(&self) -> Vec<Variable> {
        let mut vars = Vec::new();

        if let Some(result) = &self.skill_result {
            vars.push(Variable {
                name: "_skill_name".to_string(),
                value: format!("\"{}\"", result.trace.skill_name),
                type_name: Some("string".to_string()),
                variables_reference: 0,
            });

            vars.push(Variable {
                name: "_session_id".to_string(),
                value: format!("\"{}\"", result.trace.session_id),
                type_name: Some("string".to_string()),
                variables_reference: 0,
            });

            vars.push(Variable {
                name: "_final_prompt_length".to_string(),
                value: result.final_prompt.len().to_string(),
                type_name: Some("number".to_string()),
                variables_reference: 0,
            });

            vars.push(Variable {
                name: "_completed".to_string(),
                value: result.trace.completed.to_string(),
                type_name: Some("boolean".to_string()),
                variables_reference: 0,
            });

            vars.push(Variable {
                name: "_steps_count".to_string(),
                value: result.trace.steps.len().to_string(),
                type_name: Some("number".to_string()),
                variables_reference: 0,
            });
        }

        vars
    }

    /// Get command execution variables.
    fn get_command_variables(&self) -> Vec<Variable> {
        let mut vars = Vec::new();

        if let Some(result) = &self.skill_result {
            for (i, cmd) in result.commands_executed.iter().enumerate() {
                let status = if cmd.exit_code == 0 { "✓" } else { "✗" };
                vars.push(Variable {
                    name: format!("command_{}", i),
                    value: format!("{} `{}` → exit {}", status, cmd.command, cmd.exit_code),
                    type_name: Some("command".to_string()),
                    variables_reference: 0,
                });

                if !cmd.stdout.is_empty() {
                    let preview = if cmd.stdout.len() > 100 {
                        format!("{}...", &cmd.stdout[..100])
                    } else {
                        cmd.stdout.clone()
                    };
                    vars.push(Variable {
                        name: format!("command_{}_stdout", i),
                        value: format!("\"{}\"", preview.replace('\n', "\\n")),
                        type_name: Some("string".to_string()),
                        variables_reference: 0,
                    });
                }

                if !cmd.stderr.is_empty() {
                    let preview = if cmd.stderr.len() > 100 {
                        format!("{}...", &cmd.stderr[..100])
                    } else {
                        cmd.stderr.clone()
                    };
                    vars.push(Variable {
                        name: format!("command_{}_stderr", i),
                        value: format!("\"{}\"", preview.replace('\n', "\\n")),
                        type_name: Some("string".to_string()),
                        variables_reference: 0,
                    });
                }

                vars.push(Variable {
                    name: format!("command_{}_duration_ms", i),
                    value: cmd.duration_ms.to_string(),
                    type_name: Some("number".to_string()),
                    variables_reference: 0,
                });
            }
        }

        vars
    }

    /// Get reference file variables.
    fn get_reference_variables(&self) -> Vec<Variable> {
        let mut vars = Vec::new();

        if let Some(result) = &self.skill_result {
            for reference in &result.references_loaded {
                vars.push(Variable {
                    name: format!("[{}]", reference.link_text),
                    value: format!(
                        "{} ({} lines)",
                        reference.path.display(),
                        reference.line_count
                    ),
                    type_name: Some("reference".to_string()),
                    variables_reference: 0,
                });
            }
        }

        vars
    }
}

/// A variable in the debug view.
#[derive(Debug, Clone, Serialize)]
pub struct Variable {
    pub name: String,
    pub value: String,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    #[serde(rename = "variablesReference")]
    pub variables_reference: i64,
}

/// A breakpoint.
#[derive(Debug, Clone, Serialize)]
pub struct Breakpoint {
    pub id: Option<i64>,
    pub verified: bool,
    pub line: Option<u32>,
    pub message: Option<String>,
}

/// A stack frame.
#[derive(Debug, Clone, Serialize)]
pub struct StackFrame {
    pub id: i64,
    pub name: String,
    pub source: Option<Source>,
    pub line: u32,
    pub column: u32,
}

/// A source file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub name: Option<String>,
    pub path: Option<String>,
}

/// A thread.
#[derive(Debug, Clone, Serialize)]
pub struct Thread {
    pub id: i64,
    pub name: String,
}

/// A scope.
#[derive(Debug, Clone, Serialize)]
pub struct Scope {
    pub name: String,
    pub variables_reference: i64,
    pub expensive: bool,
}

/// Format a JSON value for display.
fn format_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => format!("\"{}\"", s),
        Value::Array(arr) => format!("[{} items]", arr.len()),
        Value::Object(obj) => format!("{{{} properties}}", obj.len()),
    }
}

/// Get the type name of a JSON value.
fn value_type(value: &Value) -> String {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
    .to_string()
}

/// Run the DAP server on stdin/stdout.
pub async fn run_server() {
    let server = DebugServer::new();
    server.run().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_debug_session_default() {
        let session = DebugSession::default();
        assert!(session.source_path.is_none());
        assert!(session.breakpoints.is_empty());
        assert_eq!(session.state, ExecutionState::NotStarted);
        assert!(session.render_result.is_none());
    }

    #[test]
    fn test_debug_session_has_breakpoint() {
        let mut session = DebugSession::default();
        session.breakpoints.push(BreakpointLocation {
            line: 5,
            condition: None,
            node_id: None,
        });
        session.breakpoints.push(BreakpointLocation {
            line: 10,
            condition: Some("debug".to_string()),
            node_id: None,
        });

        assert!(session.has_breakpoint(5));
        assert!(session.has_breakpoint(10));
        assert!(!session.has_breakpoint(7));
    }

    #[test]
    fn test_debug_session_check_conditional_breakpoint() {
        let mut session = DebugSession {
            context: json!({ "debug": true, "disabled": false }),
            ..DebugSession::default()
        };

        // Unconditional breakpoint
        session.breakpoints.push(BreakpointLocation {
            line: 5,
            condition: None,
            node_id: None,
        });

        // Conditional breakpoint that should trigger
        session.breakpoints.push(BreakpointLocation {
            line: 10,
            condition: Some("debug".to_string()),
            node_id: None,
        });

        // Conditional breakpoint that should NOT trigger
        session.breakpoints.push(BreakpointLocation {
            line: 15,
            condition: Some("disabled".to_string()),
            node_id: None,
        });

        assert!(session.check_breakpoint(5)); // No condition
        assert!(session.check_breakpoint(10)); // debug is true
        assert!(!session.check_breakpoint(15)); // disabled is false
        assert!(!session.check_breakpoint(20)); // No breakpoint
    }

    #[test]
    fn test_debug_session_get_variables() {
        let session = DebugSession {
            context: json!({
                "name": "Alice",
                "count": 42,
                "enabled": true
            }),
            ..DebugSession::default()
        };

        let vars = session.get_variables();

        assert!(vars
            .iter()
            .any(|v| v.name == "name" && v.value.contains("Alice")));
        assert!(vars.iter().any(|v| v.name == "count" && v.value == "42"));
        assert!(vars
            .iter()
            .any(|v| v.name == "enabled" && v.value == "true"));
    }

    #[test]
    fn test_debug_session_get_value_by_path() {
        let session = DebugSession {
            context: json!({
                "user": {
                    "name": "Bob",
                    "settings": {
                        "theme": "dark"
                    }
                }
            }),
            ..DebugSession::default()
        };

        assert_eq!(session.get_value_by_path("user.name"), Some(&json!("Bob")));
        assert_eq!(
            session.get_value_by_path("user.settings.theme"),
            Some(&json!("dark"))
        );
        assert_eq!(session.get_value_by_path("missing"), None);
    }

    #[test]
    fn test_format_value() {
        assert_eq!(format_value(&json!(null)), "null");
        assert_eq!(format_value(&json!(true)), "true");
        assert_eq!(format_value(&json!(42)), "42");
        assert_eq!(format_value(&json!("hello")), "\"hello\"");
        assert_eq!(format_value(&json!([1, 2, 3])), "[3 items]");
        assert_eq!(format_value(&json!({"a": 1, "b": 2})), "{2 properties}");
    }

    #[test]
    fn test_value_type() {
        assert_eq!(value_type(&json!(null)), "null");
        assert_eq!(value_type(&json!(true)), "boolean");
        assert_eq!(value_type(&json!(42)), "number");
        assert_eq!(value_type(&json!("hello")), "string");
        assert_eq!(value_type(&json!([1, 2])), "array");
        assert_eq!(value_type(&json!({"a": 1})), "object");
    }

    #[test]
    fn test_execution_state_transitions() {
        let not_started = ExecutionState::NotStarted;
        let running = ExecutionState::Running;
        let paused = ExecutionState::Paused {
            line: 5,
            reason: "breakpoint".to_string(),
        };
        let stopped = ExecutionState::Stopped {
            reason: "completed".to_string(),
        };

        assert_eq!(not_started, ExecutionState::NotStarted);
        assert_eq!(running, ExecutionState::Running);
        assert!(matches!(paused, ExecutionState::Paused { line: 5, .. }));
        assert!(matches!(stopped, ExecutionState::Stopped { .. }));
    }
}

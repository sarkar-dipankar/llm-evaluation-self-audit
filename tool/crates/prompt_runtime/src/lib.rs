//! LLM execution and trace instrumentation for PromptDbg.
//!
//! This crate handles:
//! - Template evaluation with detailed tracing
//! - LLM provider communication
//! - Trace collection and breakpoint handling
//! - IR-based execution mapping
//! - Telemetry and metrics collection

use std::collections::HashMap;

use prompt_ir::{PromptIR, PromptTrace, ProviderConfig, TraceStep, TraceStepKind};
use prompt_template::RenderResult;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub mod cache;
pub mod coverage;
pub mod diff;
pub mod lint;
pub mod mutate;
pub mod provider;
pub mod skill_executor;
pub mod telemetry;

pub use cache::{
    global_cache, init_global_cache, Cache, CacheConfig, CacheStats, CachedIR, CachedResponse,
    ContentHash,
};
pub use coverage::{compute_coverage, BranchCoverage, CoverageReport};
pub use diff::{
    behavior_signature, diff_traces, llm_judge_killed, resolve_applied, BehaviorSignature, Oracle,
    OutputOracle, TraceDiff, TraceOracle,
};
pub use lint::unreachable_rules;
pub use mutate::{
    generate_mutants, run_mutation_testing, run_mutation_testing_default, Mutant, MutantStatus,
    MutationOperator, MutationReport, OperatorStats,
};
pub use provider::ProviderClient;
pub use skill_executor::{
    execute_skill, CommandBreakpoint, CommandOutput, LoadedReference, SkillExecutionConfig,
    SkillExecutionResult, SkillExecutor, SkillStepKind, SkillTrace, SkillTraceStep,
};
pub use telemetry::{
    global_metrics, BreakpointMetrics, LintMetrics, MetricsCollector, MetricsSummary,
    ProviderMetrics, ProviderStats, TemplateMetrics,
};

#[derive(Error, Debug)]
pub enum RuntimeError {
    #[error("Template error at line {line}: {message}")]
    TemplateErrorAt {
        line: u32,
        column: u32,
        message: String,
    },

    #[error("Template error: {0}")]
    TemplateError(#[from] prompt_template::TemplateError),

    #[error("Provider error: {message}\n\nTroubleshooting:\n{hint}")]
    ProviderError { message: String, hint: String },

    #[error("Connection error: {message}\n\nCheck your network connection and provider endpoint.")]
    ConnectionError { message: String },

    #[error("Request timeout after {timeout_ms}ms\n\nThe provider took too long to respond. Try:\n  - Increasing timeout_ms in config\n  - Using a smaller model\n  - Simplifying your prompt")]
    Timeout { timeout_ms: u64 },

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Breakpoint hit: {node_id} at line {line}")]
    BreakpointHit { node_id: String, line: u32 },

    #[error("Command execution failed: {command}\n{message}")]
    CommandExecutionError { command: String, message: String },

    #[error("Command timeout after {timeout_ms}ms: {command}")]
    CommandTimeout { command: String, timeout_ms: u64 },
}

impl RuntimeError {
    pub fn provider_error(message: impl Into<String>) -> Self {
        let msg = message.into();
        let hint = if msg.contains("401") || msg.contains("Unauthorized") {
            "Check that your API key is valid and has not expired."
        } else if msg.contains("429") || msg.contains("rate limit") {
            "You've hit rate limits. Wait a moment and try again."
        } else if msg.contains("500") || msg.contains("503") {
            "The provider is experiencing issues. Try again later."
        } else {
            "Check the provider's API documentation for details."
        };
        Self::ProviderError {
            message: msg,
            hint: hint.to_string(),
        }
    }

    pub fn connection_error(message: impl Into<String>) -> Self {
        Self::ConnectionError {
            message: message.into(),
        }
    }
}

pub type RuntimeResult<T> = Result<T, RuntimeError>;

/// Debug session configuration.
#[derive(Debug, Clone)]
pub struct DebugConfig {
    /// Template context (variables)
    pub context: Value,
    /// Breakpoint locations (line numbers, 1-based)
    pub breakpoints: Vec<BreakpointLocation>,
    /// Whether to run in template-only mode
    pub template_only: bool,
    /// Stop on entry (before any execution)
    pub stop_on_entry: bool,
}

/// A breakpoint location.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreakpointLocation {
    /// Line number (1-based)
    pub line: u32,
    /// Optional condition expression
    pub condition: Option<String>,
    /// Optional node ID to break on
    pub node_id: Option<String>,
}

impl BreakpointLocation {
    pub fn at_line(line: u32) -> Self {
        Self {
            line,
            condition: None,
            node_id: None,
        }
    }

    pub fn with_condition(mut self, condition: impl Into<String>) -> Self {
        self.condition = Some(condition.into());
        self
    }

    pub fn with_node(mut self, node_id: impl Into<String>) -> Self {
        self.node_id = Some(node_id.into());
        self
    }
}

impl Default for DebugConfig {
    fn default() -> Self {
        Self {
            context: Value::Object(Default::default()),
            breakpoints: Vec::new(),
            template_only: false,
            stop_on_entry: false,
        }
    }
}

/// Detailed execution result with trace information.
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    /// The final trace
    pub trace: PromptTrace,
    /// Template render result
    pub render_result: RenderResult,
    /// LLM response (if not template-only)
    pub llm_response: Option<String>,
    /// Node-to-line mapping
    pub node_line_map: HashMap<String, u32>,
    /// Line-to-node mapping
    pub line_node_map: HashMap<u32, Vec<String>>,
    /// Breakpoints that were hit
    pub breakpoints_hit: Vec<BreakpointHit>,
}

/// Information about a breakpoint that was hit.
#[derive(Debug, Clone)]
pub struct BreakpointHit {
    /// The breakpoint that was hit
    pub location: BreakpointLocation,
    /// The trace step index when hit
    pub step_index: usize,
    /// Current variable values at breakpoint
    pub variables: HashMap<String, Value>,
}

/// Execute a prompt and collect traces.
pub async fn execute(
    content: &str,
    ir: &PromptIR,
    provider_config: &ProviderConfig,
    debug_config: &DebugConfig,
) -> RuntimeResult<PromptTrace> {
    let result = execute_with_details(content, ir, provider_config, debug_config).await?;
    Ok(result.trace)
}

/// Execute a prompt with detailed execution information.
pub async fn execute_with_details(
    content: &str,
    ir: &PromptIR,
    provider_config: &ProviderConfig,
    debug_config: &DebugConfig,
) -> RuntimeResult<ExecutionResult> {
    let session_id = generate_session_id();
    let mut trace = PromptTrace::new(&session_id);
    let mut breakpoints_hit = Vec::new();

    // Build node-to-line mappings from IR
    let (node_line_map, line_node_map) = build_line_mappings(ir);

    // Step 0: Planning
    trace.add_step(
        TraceStep::new(0, TraceStepKind::Plan)
            .with_explanation("Analyzing prompt structure")
            .with_rules_used(ir.rules().iter().map(|r| r.id.clone()).collect()),
    );

    // Check stop-on-entry
    if debug_config.stop_on_entry {
        if let Some(bp) = check_breakpoint(1, &debug_config.breakpoints, &debug_config.context) {
            breakpoints_hit.push(BreakpointHit {
                location: bp.clone(),
                step_index: 0,
                variables: extract_variables(&debug_config.context),
            });
        }
    }

    // Step 1: Template evaluation
    trace.add_step(
        TraceStep::new(1, TraceStepKind::Execution)
            .with_explanation("Evaluating template with provided context"),
    );

    let render_result = prompt_template::render(content, &debug_config.context)?;

    // Add trace steps for each branch decision
    for branch in render_result.branches_taken.iter() {
        let step_idx = (trace.steps.len()) as u32;
        let node_ids = line_node_map
            .get(&(branch.condition_line + 1))
            .cloned()
            .unwrap_or_default();

        trace.add_step(
            TraceStep::new(step_idx, TraceStepKind::Execution)
                .with_explanation(format!(
                    "Branch '{}': {} → {}",
                    branch.condition,
                    branch.evaluated_value.as_deref().unwrap_or("?"),
                    branch.branch_executed
                ))
                .with_rules_used(node_ids.clone()),
        );

        // Check for breakpoints on this line
        if let Some(bp) = check_breakpoint(
            branch.condition_line + 1,
            &debug_config.breakpoints,
            &debug_config.context,
        ) {
            breakpoints_hit.push(BreakpointHit {
                location: bp.clone(),
                step_index: trace.steps.len() - 1,
                variables: extract_variables(&debug_config.context),
            });
        }
    }

    // Add trace steps for loops
    for loop_info in &render_result.loop_iterations {
        let step_idx = (trace.steps.len()) as u32;
        trace.add_step(
            TraceStep::new(step_idx, TraceStepKind::Execution).with_explanation(format!(
                "Loop 'for {} in {}': {} iterations",
                loop_info.variable, loop_info.collection, loop_info.iteration_count
            )),
        );
    }

    // Template rendered step. Record the rules that were actually rendered (their
    // source line fell inside a taken region) so the trace faithfully reflects which
    // rules fired, rather than relying on a positional heuristic.
    let rendered: std::collections::HashSet<u32> =
        render_result.rendered_lines.iter().copied().collect();
    let rendered_rule_ids: Vec<String> = ir
        .rules()
        .iter()
        .filter(|n| rendered.contains(&n.range.start_line))
        .map(|n| n.id.clone())
        .collect();
    let render_step_idx = (trace.steps.len()) as u32;
    trace.add_step(
        TraceStep::new(render_step_idx, TraceStepKind::Execution)
            .with_explanation("Template rendered successfully")
            .with_rules_used(rendered_rule_ids)
            .with_partial_answer(&render_result.output),
    );

    // Check for template-only mode
    if debug_config.template_only {
        trace.complete(render_result.output.clone());
        return Ok(ExecutionResult {
            trace,
            render_result,
            llm_response: None,
            node_line_map,
            line_node_map,
            breakpoints_hit,
        });
    }

    // Step 2: LLM execution, served from the content-addressed response cache when
    // available (deterministic replay + cost reduction across runs).
    let provider_name = provider_config.provider.to_string();
    let llm_step_idx = (trace.steps.len()) as u32;
    let response = if let Some(cached) = cache::global_cache().get_response(
        &render_result.output,
        &provider_name,
        &provider_config.model,
    ) {
        trace.add_step(
            TraceStep::new(llm_step_idx, TraceStepKind::Execution).with_explanation(format!(
                "Cache hit for {} ({})",
                provider_name, provider_config.model
            )),
        );
        cached
    } else {
        trace.add_step(
            TraceStep::new(llm_step_idx, TraceStepKind::Execution).with_explanation(format!(
                "Sending to {} ({})",
                provider_name, provider_config.model
            )),
        );
        let client = provider::create_client(provider_config)?;
        let r = client.complete(&render_result.output).await?;
        cache::global_cache().put_response(
            &render_result.output,
            &provider_name,
            &provider_config.model,
            r.clone(),
            None,
        );
        r
    };

    // Step 3: Collect final trace
    let final_step_idx = (trace.steps.len()) as u32;
    trace.add_step(
        TraceStep::new(final_step_idx, TraceStepKind::Summary)
            .with_explanation("Execution completed successfully")
            .with_rules_used(ir.rules().iter().map(|r| r.id.clone()).collect()),
    );

    trace.complete(response.clone());

    Ok(ExecutionResult {
        trace,
        render_result,
        llm_response: Some(response),
        node_line_map,
        line_node_map,
        breakpoints_hit,
    })
}

/// Build mappings between IR nodes and source lines.
fn build_line_mappings(ir: &PromptIR) -> (HashMap<String, u32>, HashMap<u32, Vec<String>>) {
    let mut node_to_line = HashMap::new();
    let mut line_to_nodes: HashMap<u32, Vec<String>> = HashMap::new();

    for node in &ir.nodes {
        let line = node.range.start_line + 1; // Convert to 1-based
        node_to_line.insert(node.id.clone(), line);
        line_to_nodes.entry(line).or_default().push(node.id.clone());
    }

    (node_to_line, line_to_nodes)
}

/// Check if a breakpoint should trigger at the given line.
fn check_breakpoint<'a>(
    line: u32,
    breakpoints: &'a [BreakpointLocation],
    context: &Value,
) -> Option<&'a BreakpointLocation> {
    for bp in breakpoints {
        if bp.line == line {
            // Check condition if present
            if let Some(condition) = &bp.condition {
                if !evaluate_breakpoint_condition(condition, context) {
                    continue;
                }
            }
            return Some(bp);
        }
    }
    None
}

/// Evaluate a breakpoint condition expression.
fn evaluate_breakpoint_condition(condition: &str, context: &Value) -> bool {
    // Simple evaluation - check if variable exists and is truthy
    if let Some(value) = get_value_by_path(context, condition) {
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

/// Get a value from context by dot-separated path.
fn get_value_by_path<'a>(context: &'a Value, path: &str) -> Option<&'a Value> {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = context;

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

/// Extract variables from context for breakpoint inspection.
fn extract_variables(context: &Value) -> HashMap<String, Value> {
    let mut vars = HashMap::new();
    if let Value::Object(map) = context {
        for (key, value) in map {
            vars.insert(key.clone(), value.clone());
        }
    }
    vars
}

/// Execute template only (no LLM call).
pub fn execute_template_only(content: &str, context: &Value) -> RuntimeResult<RenderResult> {
    Ok(prompt_template::render(content, context)?)
}

/// Stepwise executor for DAP integration.
/// Allows pausing and resuming execution at breakpoints.
pub struct StepwiseExecutor {
    content: String,
    ir: PromptIR,
    provider_config: ProviderConfig,
    debug_config: DebugConfig,
    current_step: usize,
    trace: PromptTrace,
    render_result: Option<RenderResult>,
    state: ExecutorState,
    node_line_map: HashMap<String, u32>,
    line_node_map: HashMap<u32, Vec<String>>,
}

/// State of the stepwise executor.
#[derive(Debug, Clone, PartialEq)]
pub enum ExecutorState {
    /// Not started
    NotStarted,
    /// Paused at a step
    Paused { step: usize, line: u32 },
    /// Running (between steps)
    Running,
    /// Completed successfully
    Completed,
    /// Failed with error
    Failed { error: String },
}

impl StepwiseExecutor {
    /// Create a new stepwise executor.
    pub fn new(
        content: String,
        ir: PromptIR,
        provider_config: ProviderConfig,
        debug_config: DebugConfig,
    ) -> Self {
        let session_id = generate_session_id();
        let (node_line_map, line_node_map) = build_line_mappings(&ir);

        Self {
            content,
            ir,
            provider_config,
            debug_config,
            current_step: 0,
            trace: PromptTrace::new(&session_id),
            render_result: None,
            state: ExecutorState::NotStarted,
            node_line_map,
            line_node_map,
        }
    }

    /// Get current execution state.
    pub fn state(&self) -> &ExecutorState {
        &self.state
    }

    /// Get current step index.
    pub fn current_step(&self) -> usize {
        self.current_step
    }

    /// Get the IR being executed.
    pub fn ir(&self) -> &PromptIR {
        &self.ir
    }

    /// Get the mapping from source lines to IR node IDs at that line.
    pub fn line_node_map(&self) -> &HashMap<u32, Vec<String>> {
        &self.line_node_map
    }

    /// Get the current trace.
    pub fn trace(&self) -> &PromptTrace {
        &self.trace
    }

    /// Get the render result (if template has been executed).
    pub fn render_result(&self) -> Option<&RenderResult> {
        self.render_result.as_ref()
    }

    /// Get line number for current step.
    pub fn current_line(&self) -> u32 {
        if let Some(step) = self.trace.steps.get(self.current_step) {
            // Try to find line from rules_used
            for node_id in &step.rules_used {
                if let Some(line) = self.node_line_map.get(node_id) {
                    return *line;
                }
            }
        }
        // Default to step index + 1
        (self.current_step as u32).saturating_add(1)
    }

    /// Check if current line has a breakpoint.
    pub fn has_breakpoint_at_current(&self) -> bool {
        let line = self.current_line();
        self.debug_config
            .breakpoints
            .iter()
            .any(|bp| bp.line == line)
    }

    /// Step to the next execution point.
    pub async fn step(&mut self) -> RuntimeResult<bool> {
        match &self.state {
            ExecutorState::NotStarted => {
                self.start().await?;
                Ok(true)
            }
            ExecutorState::Paused { .. } | ExecutorState::Running => self.advance().await,
            ExecutorState::Completed | ExecutorState::Failed { .. } => Ok(false),
        }
    }

    /// Continue running until breakpoint or completion.
    pub async fn continue_execution(&mut self) -> RuntimeResult<()> {
        loop {
            if !self.step().await? {
                break;
            }
            if self.has_breakpoint_at_current() {
                let line = self.current_line();
                self.state = ExecutorState::Paused {
                    step: self.current_step,
                    line,
                };
                break;
            }
        }
        Ok(())
    }

    /// Start execution.
    async fn start(&mut self) -> RuntimeResult<()> {
        self.state = ExecutorState::Running;

        // Add initial step
        self.trace.add_step(
            TraceStep::new(0, TraceStepKind::Plan).with_explanation("Starting execution"),
        );

        // Render template
        match prompt_template::render(&self.content, &self.debug_config.context) {
            Ok(result) => {
                self.trace.add_step(
                    TraceStep::new(1, TraceStepKind::Execution)
                        .with_explanation("Template rendered")
                        .with_partial_answer(&result.output),
                );
                self.render_result = Some(result);
                self.current_step = 1;
            }
            Err(e) => {
                self.state = ExecutorState::Failed {
                    error: e.to_string(),
                };
                return Err(e.into());
            }
        }

        if self.debug_config.stop_on_entry {
            self.state = ExecutorState::Paused { step: 0, line: 1 };
        }

        Ok(())
    }

    /// Advance to next step.
    async fn advance(&mut self) -> RuntimeResult<bool> {
        self.current_step += 1;

        // Check if we should call LLM
        if self.current_step == 2 && !self.debug_config.template_only {
            if let Some(render) = &self.render_result {
                self.trace.add_step(
                    TraceStep::new(2, TraceStepKind::Execution).with_explanation(format!(
                        "Calling {} ({})",
                        self.provider_config.provider, self.provider_config.model
                    )),
                );

                let client = provider::create_client(&self.provider_config)?;
                match client.complete(&render.output).await {
                    Ok(response) => {
                        self.trace.add_step(
                            TraceStep::new(3, TraceStepKind::Summary)
                                .with_explanation("Completed")
                                .with_partial_answer(&response),
                        );
                        self.trace.complete(response);
                        self.state = ExecutorState::Completed;
                        return Ok(false);
                    }
                    Err(e) => {
                        self.state = ExecutorState::Failed {
                            error: e.to_string(),
                        };
                        return Err(e);
                    }
                }
            }
        }

        // Template-only completion
        if self.debug_config.template_only && self.current_step >= 2 {
            if let Some(render) = &self.render_result {
                self.trace.complete(render.output.clone());
            }
            self.state = ExecutorState::Completed;
            return Ok(false);
        }

        self.state = ExecutorState::Paused {
            step: self.current_step,
            line: self.current_line(),
        };
        Ok(true)
    }

    /// Get variables at current scope.
    pub fn get_variables(&self) -> HashMap<String, Value> {
        let mut vars = extract_variables(&self.debug_config.context);

        // Add render info if available
        if let Some(render) = &self.render_result {
            vars.insert(
                "_rendered_output".to_string(),
                Value::String(render.output.clone()),
            );
            vars.insert(
                "_variables_used".to_string(),
                Value::Array(
                    render
                        .variables_used
                        .iter()
                        .map(|v| Value::String(v.clone()))
                        .collect(),
                ),
            );
        }

        vars
    }
}

fn generate_session_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("sess_{:x}", timestamp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_template_only_execution() {
        let content = "Hello {{ name }}!";
        let context = json!({ "name": "World" });
        let result = execute_template_only(content, &context).unwrap();
        assert_eq!(result.output, "Hello World!");
    }

    #[test]
    fn test_breakpoint_location() {
        let bp = BreakpointLocation::at_line(5)
            .with_condition("debug_mode")
            .with_node("node_1");

        assert_eq!(bp.line, 5);
        assert_eq!(bp.condition, Some("debug_mode".to_string()));
        assert_eq!(bp.node_id, Some("node_1".to_string()));
    }

    #[test]
    fn test_breakpoint_condition_evaluation() {
        let context = json!({
            "debug": true,
            "count": 5,
            "empty": "",
            "items": [1, 2, 3]
        });

        assert!(evaluate_breakpoint_condition("debug", &context));
        assert!(evaluate_breakpoint_condition("count", &context));
        assert!(!evaluate_breakpoint_condition("empty", &context));
        assert!(evaluate_breakpoint_condition("items", &context));
        assert!(!evaluate_breakpoint_condition("missing", &context));
    }

    #[test]
    fn test_extract_variables() {
        let context = json!({
            "name": "Alice",
            "count": 42
        });

        let vars = extract_variables(&context);
        assert_eq!(vars.get("name"), Some(&Value::String("Alice".to_string())));
        assert_eq!(vars.get("count"), Some(&json!(42)));
    }
}

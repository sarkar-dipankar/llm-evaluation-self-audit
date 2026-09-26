//! DAP server implementation.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use prompt_ir::{TraceStep, TraceStepKind};
use prompt_runtime::{
    BreakpointLocation, CommandBreakpoint, SkillExecutionConfig, SkillExecutor, SkillStepKind,
    SkillTrace,
};
use prompt_template::skill_directory::SkillDirectory;
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tracing::{debug, error, info};

use crate::protocol::*;
use crate::{
    Breakpoint, DebugSession, ExecutionState, Scope, SkillDebugPhase, Source, StackFrame, Thread,
};

/// The PromptDbg debug adapter server.
pub struct DebugServer {
    session: Arc<RwLock<DebugSession>>,
}

impl DebugServer {
    pub fn new() -> Self {
        Self {
            session: Arc::new(RwLock::new(DebugSession::default())),
        }
    }

    /// Run the server on stdin/stdout.
    pub async fn run(&self) {
        let stdin = std::io::stdin();
        let mut stdout = std::io::stdout();
        let reader = BufReader::new(stdin.lock());
        let mut lines = reader.lines();

        while let Some(Ok(header)) = lines.next() {
            // Read Content-Length header
            if !header.starts_with("Content-Length:") {
                continue;
            }

            let content_length: usize = header
                .trim_start_matches("Content-Length:")
                .trim()
                .parse()
                .unwrap_or(0);

            // Skip empty line
            let _ = lines.next();

            // Read content
            let mut content = vec![0u8; content_length];
            if std::io::stdin().read_exact(&mut content).is_err() {
                break;
            }

            let content_str = String::from_utf8_lossy(&content);
            debug!("Received: {}", content_str);

            // Parse request
            let request: Request = match serde_json::from_str(&content_str) {
                Ok(req) => req,
                Err(e) => {
                    error!("Failed to parse request: {}", e);
                    continue;
                }
            };

            // Handle request
            let responses = self.handle_request(&request).await;

            // Send responses
            for response in responses {
                let response_str = serde_json::to_string(&response).unwrap();
                let output = format!(
                    "Content-Length: {}\r\n\r\n{}",
                    response_str.len(),
                    response_str
                );
                debug!("Sending: {}", response_str);
                let _ = stdout.write_all(output.as_bytes());
                let _ = stdout.flush();
            }
        }
    }

    /// Handle a DAP request.
    async fn handle_request(&self, request: &Request) -> Vec<Value> {
        let mut responses = Vec::new();

        match request.command.as_str() {
            "initialize" => {
                let caps = Capabilities::promptdbg();
                responses.push(
                    serde_json::to_value(Response::success(
                        request,
                        Some(serde_json::to_value(caps).unwrap()),
                    ))
                    .unwrap(),
                );

                // Send initialized event
                responses.push(serde_json::to_value(Event::initialized()).unwrap());
            }

            "launch" => {
                if let Some(args) = &request.arguments {
                    match serde_json::from_value::<LaunchArguments>(args.clone()) {
                        Ok(launch_args) => {
                            let result = self.handle_launch(launch_args).await;
                            match result {
                                Ok(_) => {
                                    responses.push(
                                        serde_json::to_value(Response::success(request, None))
                                            .unwrap(),
                                    );
                                }
                                Err(e) => {
                                    responses.push(
                                        serde_json::to_value(Response::error(request, &e)).unwrap(),
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            responses.push(
                                serde_json::to_value(Response::error(
                                    request,
                                    &format!("Invalid launch arguments: {}", e),
                                ))
                                .unwrap(),
                            );
                        }
                    }
                }
            }

            "setBreakpoints" => {
                if let Some(args) = &request.arguments {
                    match serde_json::from_value::<SetBreakpointsArguments>(args.clone()) {
                        Ok(bp_args) => {
                            let breakpoints = self.handle_set_breakpoints(bp_args).await;
                            responses.push(
                                serde_json::to_value(Response::success(
                                    request,
                                    Some(json!({ "breakpoints": breakpoints })),
                                ))
                                .unwrap(),
                            );
                        }
                        Err(e) => {
                            responses.push(
                                serde_json::to_value(Response::error(
                                    request,
                                    &format!("Invalid breakpoints: {}", e),
                                ))
                                .unwrap(),
                            );
                        }
                    }
                }
            }

            "configurationDone" => {
                responses.push(serde_json::to_value(Response::success(request, None)).unwrap());

                // Start execution
                let session = self.session.read().await;
                if session.source_content.is_some() {
                    drop(session);

                    // Send stopped event if has breakpoint at line 1
                    let session = self.session.read().await;
                    if session.has_breakpoint(1) {
                        responses.push(
                            serde_json::to_value(Event::stopped(
                                "entry",
                                1,
                                Some("Stopped on entry"),
                            ))
                            .unwrap(),
                        );
                    }
                }
            }

            "threads" => {
                let threads = vec![Thread {
                    id: 1,
                    name: "Prompt Execution".to_string(),
                }];
                responses.push(
                    serde_json::to_value(Response::success(
                        request,
                        Some(json!({ "threads": threads })),
                    ))
                    .unwrap(),
                );
            }

            "stackTrace" => {
                let session = self.session.read().await;
                let frames = self.get_stack_frames(&session);
                responses.push(
                    serde_json::to_value(Response::success(
                        request,
                        Some(json!({
                            "stackFrames": frames,
                            "totalFrames": frames.len(),
                        })),
                    ))
                    .unwrap(),
                );
            }

            "scopes" => {
                let session = self.session.read().await;
                let mut scopes = vec![
                    Scope {
                        name: "Context".to_string(),
                        variables_reference: 1,
                        expensive: false,
                    },
                    Scope {
                        name: "Trace".to_string(),
                        variables_reference: 2,
                        expensive: false,
                    },
                ];

                // Add skill-specific scopes
                if session.is_skill {
                    scopes.push(Scope {
                        name: "Skill".to_string(),
                        variables_reference: 3,
                        expensive: false,
                    });
                    scopes.push(Scope {
                        name: "Commands".to_string(),
                        variables_reference: 4,
                        expensive: false,
                    });
                    scopes.push(Scope {
                        name: "References".to_string(),
                        variables_reference: 5,
                        expensive: false,
                    });
                }

                responses.push(
                    serde_json::to_value(Response::success(
                        request,
                        Some(json!({ "scopes": scopes })),
                    ))
                    .unwrap(),
                );
            }

            "variables" => {
                if let Some(args) = &request.arguments {
                    let vars_ref = args
                        .get("variablesReference")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let session = self.session.read().await;
                    let variables = if vars_ref > 0 {
                        session.get_variables_for_scope(vars_ref)
                    } else {
                        session.get_variables()
                    };
                    responses.push(
                        serde_json::to_value(Response::success(
                            request,
                            Some(json!({ "variables": variables })),
                        ))
                        .unwrap(),
                    );
                } else {
                    let session = self.session.read().await;
                    let variables = session.get_variables();
                    responses.push(
                        serde_json::to_value(Response::success(
                            request,
                            Some(json!({ "variables": variables })),
                        ))
                        .unwrap(),
                    );
                }
            }

            "continue" => {
                responses.push(serde_json::to_value(Response::success(request, None)).unwrap());

                // Resume execution
                {
                    let mut session = self.session.write().await;
                    session.state = ExecutionState::Running;
                }

                // Simulate running to next breakpoint or end
                let stopped = self.run_to_breakpoint_or_end().await;
                if let Some(event) = stopped {
                    responses.push(serde_json::to_value(event).unwrap());
                }
            }

            "next" | "stepIn" | "stepOut" => {
                responses.push(serde_json::to_value(Response::success(request, None)).unwrap());

                // Step to next line
                {
                    let mut session = self.session.write().await;
                    if session.current_step < session.trace.len().saturating_sub(1) {
                        session.current_step += 1;
                        let line = session.current_line();
                        session.state = ExecutionState::Paused {
                            line,
                            reason: "step".to_string(),
                        };
                    } else {
                        session.state = ExecutionState::Stopped {
                            reason: "completed".to_string(),
                        };
                    }
                }

                let session = self.session.read().await;
                match &session.state {
                    ExecutionState::Paused { .. } => {
                        responses
                            .push(serde_json::to_value(Event::stopped("step", 1, None)).unwrap());
                    }
                    ExecutionState::Stopped { .. } => {
                        responses.push(serde_json::to_value(Event::terminated()).unwrap());
                    }
                    _ => {}
                }
            }

            "evaluate" => {
                if let Some(args) = &request.arguments {
                    match serde_json::from_value::<EvaluateArguments>(args.clone()) {
                        Ok(eval_args) => {
                            let result = self.handle_evaluate(eval_args).await;
                            responses.push(
                                serde_json::to_value(Response::success(
                                    request,
                                    Some(json!({
                                        "result": result,
                                        "variablesReference": 0,
                                    })),
                                ))
                                .unwrap(),
                            );
                        }
                        Err(_) => {
                            responses.push(
                                serde_json::to_value(Response::success(
                                    request,
                                    Some(json!({
                                        "result": "",
                                        "variablesReference": 0,
                                    })),
                                ))
                                .unwrap(),
                            );
                        }
                    }
                }
            }

            "disconnect" | "terminate" => {
                responses.push(serde_json::to_value(Response::success(request, None)).unwrap());
                responses.push(serde_json::to_value(Event::terminated()).unwrap());
            }

            _ => {
                responses.push(
                    serde_json::to_value(Response::error(
                        request,
                        &format!("Unknown command: {}", request.command),
                    ))
                    .unwrap(),
                );
            }
        }

        responses
    }

    /// Handle launch request.
    async fn handle_launch(&self, args: LaunchArguments) -> Result<(), String> {
        let path = PathBuf::from(&args.program);

        // Detect if this is a skill file
        let is_skill = path
            .file_name()
            .map(|n| n.to_string_lossy() == "SKILL.md")
            .unwrap_or(false);

        if is_skill {
            self.handle_skill_launch(&path, &args).await
        } else {
            self.handle_template_launch(&path, &args).await
        }
    }

    /// Handle launching a skill file for debugging.
    async fn handle_skill_launch(&self, path: &Path, args: &LaunchArguments) -> Result<(), String> {
        // Get skill directory (parent of SKILL.md)
        let skill_dir_path = path.parent().unwrap_or(path);

        // Load skill directory
        let skill_dir = SkillDirectory::load(skill_dir_path)
            .map_err(|e| format!("Failed to load skill: {}", e))?;

        info!(
            "Launching skill debug session: {}",
            skill_dir.skill_file.frontmatter.name
        );

        // Read source content
        let content =
            std::fs::read_to_string(path).map_err(|e| format!("Failed to read SKILL.md: {}", e))?;

        // Configure execution
        let config = SkillExecutionConfig {
            arguments: args.skill_args.clone().unwrap_or_default(),
            session_id: args.session_id.clone(),
            working_directory: skill_dir_path.to_path_buf(),
            enable_commands: !args.no_commands,
            command_timeout_ms: 30000,
            template_only: args.template_only,
            breakpoints: vec![],
        };

        // Execute skill
        let mut executor = SkillExecutor::new(skill_dir, config, None);
        let result = executor
            .execute()
            .await
            .map_err(|e| format!("Skill execution failed: {}", e))?;

        info!(
            "Skill executed: {} steps, {} commands, {} references",
            result.trace.steps.len(),
            result.commands_executed.len(),
            result.references_loaded.len()
        );

        // Convert SkillTrace to TraceSteps for DAP
        let trace_steps = self.skill_trace_to_dap_trace(&result.trace);

        // Reload skill directory for storage in session (executor consumed it)
        let skill_dir = SkillDirectory::load(skill_dir_path)
            .map_err(|e| format!("Failed to reload skill: {}", e))?;

        // Update session
        let mut session = self.session.write().await;
        session.is_skill = true;
        session.skill_dir = Some(skill_dir);
        session.skill_result = Some(result);
        session.skill_phase = SkillDebugPhase::Completed;
        session.trace = trace_steps;
        session.source_path = Some(path.to_path_buf());
        session.source_content = Some(content.clone());
        session.source_lines = content.lines().map(String::from).collect();
        session.template_only = args.template_only;
        session.stop_on_entry = args.stop_on_entry;
        session.state = ExecutionState::NotStarted;

        Ok(())
    }

    /// Handle launching a template file for debugging.
    async fn handle_template_launch(
        &self,
        path: &Path,
        args: &LaunchArguments,
    ) -> Result<(), String> {
        // Read source file
        let content =
            std::fs::read_to_string(path).map_err(|e| format!("Failed to read file: {}", e))?;

        // Load context
        let context = if let Some(ctx) = args.context.clone() {
            ctx
        } else if let Some(ref ctx_file) = args.context_file {
            let ctx_content = std::fs::read_to_string(ctx_file)
                .map_err(|e| format!("Failed to read context file: {}", e))?;
            serde_json::from_str(&ctx_content)
                .map_err(|e| format!("Failed to parse context: {}", e))?
        } else {
            Value::Object(Default::default())
        };

        // Parse source lines
        let source_lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();

        // Update session
        let mut session = self.session.write().await;
        session.source_path = Some(path.to_path_buf());
        session.source_content = Some(content.clone());
        session.source_lines = source_lines.clone();
        session.context = context.clone();
        session.template_only = args.template_only;
        session.stop_on_entry = args.stop_on_entry;
        session.state = ExecutionState::NotStarted;
        session.is_skill = false;

        // Try to render template immediately
        match prompt_template::render(&content, &context) {
            Ok(render_result) => {
                info!(
                    "Template rendered successfully: {} chars",
                    render_result.output.len()
                );

                // Generate trace steps from render result
                let mut trace_steps = Vec::new();

                // Initial step
                trace_steps.push(
                    TraceStep::new(0, TraceStepKind::Plan)
                        .with_explanation("Starting template evaluation"),
                );

                // Add step for each branch
                for (i, branch) in render_result.branches_taken.iter().enumerate() {
                    trace_steps.push(
                        TraceStep::new((i + 1) as u32, TraceStepKind::Execution).with_explanation(
                            format!(
                                "Branch '{}': {} → {}",
                                branch.condition,
                                branch.evaluated_value.as_deref().unwrap_or("?"),
                                branch.branch_executed
                            ),
                        ),
                    );
                }

                // Add step for each loop
                for loop_info in render_result.loop_iterations.iter() {
                    trace_steps.push(
                        TraceStep::new((trace_steps.len()) as u32, TraceStepKind::Execution)
                            .with_explanation(format!(
                                "Loop 'for {} in {}': {} iterations",
                                loop_info.variable, loop_info.collection, loop_info.iteration_count
                            )),
                    );
                }

                // Final step with rendered output
                trace_steps.push(
                    TraceStep::new((trace_steps.len()) as u32, TraceStepKind::Summary)
                        .with_explanation("Template rendered successfully")
                        .with_partial_answer(&render_result.output),
                );

                session.trace = trace_steps;
                session.render_result = Some(render_result);
            }
            Err(e) => {
                error!("Template render failed: {}", e);
                // Create error trace
                session.trace = vec![TraceStep::new(0, TraceStepKind::Plan)
                    .with_explanation(format!("Template error: {}", e))];
            }
        }

        Ok(())
    }

    /// Convert SkillTrace to DAP TraceSteps.
    fn skill_trace_to_dap_trace(&self, skill_trace: &SkillTrace) -> Vec<TraceStep> {
        skill_trace
            .steps
            .iter()
            .map(|step| {
                let kind = match step.kind {
                    SkillStepKind::Plan => TraceStepKind::Plan,
                    SkillStepKind::Summary => TraceStepKind::Summary,
                    _ => TraceStepKind::Execution,
                };

                let mut trace_step = TraceStep::new(step.index, kind);
                if let Some(ref explanation) = step.explanation {
                    trace_step = trace_step.with_explanation(explanation);
                }
                trace_step
            })
            .collect()
    }

    /// Handle set breakpoints request.
    async fn handle_set_breakpoints(&self, args: SetBreakpointsArguments) -> Vec<Breakpoint> {
        let mut session = self.session.write().await;
        session.breakpoints.clear();
        session.command_breakpoints.clear();

        let mut result = Vec::new();
        for (i, bp) in args.breakpoints.iter().enumerate() {
            // Check if this line has a command (for skill files)
            let is_command_line = if session.is_skill {
                session
                    .skill_dir
                    .as_ref()
                    .map(|sd| {
                        sd.skill_file
                            .command_executions
                            .iter()
                            .any(|cmd| cmd.range.start_line == bp.line.saturating_sub(1))
                    })
                    .unwrap_or(false)
            } else {
                false
            };

            // Add as command breakpoint if on a command line
            if is_command_line {
                session.command_breakpoints.push(CommandBreakpoint {
                    command_pattern: None,
                    line: Some(bp.line),
                });
            }

            // Create breakpoint location with optional condition
            let location = BreakpointLocation {
                line: bp.line,
                condition: bp.condition.clone(),
                node_id: None,
            };
            session.breakpoints.push(location);

            // Verify breakpoint is on a valid line
            let verified = bp.line > 0 && bp.line <= session.source_lines.len() as u32;
            let message = if !verified {
                Some(format!("Line {} is outside source range", bp.line))
            } else if is_command_line {
                Some("Command breakpoint".to_string())
            } else {
                bp.condition
                    .as_ref()
                    .map(|cond| format!("Conditional: {}", cond))
            };

            result.push(Breakpoint {
                id: Some(i as i64 + 1),
                verified,
                line: Some(bp.line),
                message,
            });
        }

        result
    }

    /// Get stack frames for current state.
    fn get_stack_frames(&self, session: &DebugSession) -> Vec<StackFrame> {
        let mut frames = Vec::new();

        let (line, name) = match &session.state {
            ExecutionState::Paused { line, reason } => (*line, reason.clone()),
            ExecutionState::Running => {
                let current_line = session.current_line();
                (current_line, format!("Step {}", session.current_step))
            }
            _ => (1, "Prompt".to_string()),
        };

        frames.push(StackFrame {
            id: 1,
            name,
            source: session.source_path.as_ref().map(|p| Source {
                name: p.file_name().map(|n| n.to_string_lossy().to_string()),
                path: Some(p.display().to_string()),
            }),
            line,
            column: 1,
        });

        frames
    }

    /// Handle evaluate request.
    async fn handle_evaluate(&self, args: EvaluateArguments) -> String {
        let session = self.session.read().await;

        // Look up variable in context
        if let Value::Object(map) = &session.context {
            if let Some(value) = map.get(&args.expression) {
                return crate::format_value(value);
            }
        }

        // Check if it's a path expression like "context.name"
        if args.expression.starts_with("context.") {
            let path = args.expression.strip_prefix("context.").unwrap();
            if let Value::Object(map) = &session.context {
                if let Some(value) = map.get(path) {
                    return crate::format_value(value);
                }
            }
        }

        format!("Unknown: {}", args.expression)
    }

    /// Run until breakpoint or end.
    async fn run_to_breakpoint_or_end(&self) -> Option<Event> {
        let mut session = self.session.write().await;

        while session.current_step < session.trace.len() {
            let line = session.current_line();

            // Check conditional breakpoints
            if session.check_breakpoint(line) {
                session.state = ExecutionState::Paused {
                    line,
                    reason: "breakpoint".to_string(),
                };

                // Get breakpoint info for description
                let bp_info = session
                    .breakpoints
                    .iter()
                    .find(|bp| bp.line == line)
                    .map(|bp| {
                        if let Some(cond) = &bp.condition {
                            format!("Hit conditional breakpoint: {}", cond)
                        } else {
                            "Hit breakpoint".to_string()
                        }
                    })
                    .unwrap_or_else(|| "Hit breakpoint".to_string());

                return Some(Event::stopped("breakpoint", 1, Some(&bp_info)));
            }

            session.current_step += 1;
        }

        // Execution completed
        session.state = ExecutionState::Stopped {
            reason: "completed".to_string(),
        };

        // Send output event with rendered result if available
        if let Some(render) = &session.render_result {
            // Could emit an output event here for the debug console
            debug!("Execution completed. Output: {} chars", render.output.len());
        }

        Some(Event::terminated())
    }
}

impl Default for DebugServer {
    fn default() -> Self {
        Self::new()
    }
}

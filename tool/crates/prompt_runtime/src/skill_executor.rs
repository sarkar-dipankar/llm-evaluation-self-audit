//! Skill execution and debugging for Claude Code skills.
//!
//! This module handles:
//! - $ARGUMENTS and ${CLAUDE_SESSION_ID} substitution
//! - `!command` execution with timeout
//! - Reference file loading and inclusion
//! - Execution tracing for debugging

use std::path::PathBuf;
use std::time::Duration;

use prompt_ir::ProviderConfig;
use prompt_template::skill::CommandExecution;
use prompt_template::skill_directory::{SkillDirectory, SkillDirectoryError};
use serde::{Deserialize, Serialize};

use crate::{provider, RuntimeError, RuntimeResult};

/// Trace of skill execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillTrace {
    /// Session identifier
    pub session_id: String,
    /// Skill name from frontmatter
    pub skill_name: String,
    /// Execution steps
    pub steps: Vec<SkillTraceStep>,
    /// Whether execution completed successfully
    pub completed: bool,
    /// Final answer/output (if completed)
    pub final_answer: Option<String>,
}

impl SkillTrace {
    /// Create a new skill trace.
    pub fn new(session_id: &str, skill_name: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            skill_name: skill_name.to_string(),
            steps: Vec::new(),
            completed: false,
            final_answer: None,
        }
    }

    /// Add a step to the trace.
    pub fn add_step(&mut self, step: SkillTraceStep) {
        self.steps.push(step);
    }

    /// Mark the trace as completed.
    pub fn complete(&mut self, answer: Option<String>) {
        self.completed = true;
        self.final_answer = answer;
    }
}

/// A single step in the skill execution trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillTraceStep {
    /// Step index (0-based)
    pub index: u32,
    /// Type of step
    pub kind: SkillStepKind,
    /// Timestamp (ISO 8601)
    pub timestamp: String,
    /// Human-readable explanation
    pub explanation: Option<String>,
}

impl SkillTraceStep {
    /// Create a new trace step.
    pub fn new(index: u32, kind: SkillStepKind) -> Self {
        Self {
            index,
            kind,
            timestamp: chrono_timestamp(),
            explanation: None,
        }
    }

    /// Add an explanation to the step.
    pub fn with_explanation(mut self, explanation: impl Into<String>) -> Self {
        self.explanation = Some(explanation.into());
        self
    }
}

/// Types of steps in skill execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SkillStepKind {
    /// Initial planning and analysis
    Plan,
    /// Substituting $ARGUMENTS
    ArgumentSubstitution,
    /// Substituting ${CLAUDE_SESSION_ID}
    SessionIdSubstitution,
    /// Loading reference files
    ReferenceLoading,
    /// Executing a `!command`
    CommandExecution,
    /// Building final prompt
    PromptBuild,
    /// Calling LLM provider
    LlmCall,
    /// Final summary
    Summary,
}

/// Configuration for skill execution.
#[derive(Debug, Clone)]
pub struct SkillExecutionConfig {
    /// Arguments to substitute for $ARGUMENTS
    pub arguments: String,
    /// Session ID (auto-generated if None)
    pub session_id: Option<String>,
    /// Working directory for command execution
    pub working_directory: PathBuf,
    /// Enable command execution
    pub enable_commands: bool,
    /// Command timeout in milliseconds
    pub command_timeout_ms: u64,
    /// Template-only mode (no LLM call)
    pub template_only: bool,
    /// Breakpoints for debugging
    pub breakpoints: Vec<CommandBreakpoint>,
}

impl Default for SkillExecutionConfig {
    fn default() -> Self {
        Self {
            arguments: String::new(),
            session_id: None,
            working_directory: PathBuf::from("."),
            enable_commands: true,
            command_timeout_ms: 30000,
            template_only: false,
            breakpoints: Vec::new(),
        }
    }
}

/// Breakpoint for command execution.
#[derive(Debug, Clone)]
pub struct CommandBreakpoint {
    /// Glob pattern to match command (e.g., "gh*", "git *")
    pub command_pattern: Option<String>,
    /// Line number to break on
    pub line: Option<u32>,
}

/// Output from a command execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandOutput {
    /// The command that was executed
    pub command: String,
    /// Exit code (0 = success)
    pub exit_code: i32,
    /// Standard output
    pub stdout: String,
    /// Standard error
    pub stderr: String,
    /// Duration in milliseconds
    pub duration_ms: u64,
}

/// A reference file that was loaded into the prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadedReference {
    /// Path relative to skill directory
    pub path: PathBuf,
    /// Number of lines in the file
    pub line_count: usize,
    /// The [display text] from the markdown link
    pub link_text: String,
}

/// Full execution result.
#[derive(Debug)]
pub struct SkillExecutionResult {
    /// Execution trace
    pub trace: SkillTrace,
    /// Final constructed prompt
    pub final_prompt: String,
    /// LLM response (if not template_only)
    pub llm_response: Option<String>,
    /// Commands that were executed
    pub commands_executed: Vec<CommandOutput>,
    /// Reference files that were loaded
    pub references_loaded: Vec<LoadedReference>,
}

/// Skill executor with tracing support.
pub struct SkillExecutor {
    /// Loaded skill directory
    skill_dir: SkillDirectory,
    /// Execution configuration
    config: SkillExecutionConfig,
    /// LLM provider configuration
    provider_config: Option<ProviderConfig>,
    /// Current body being built
    current_body: String,
    /// Execution trace
    trace: SkillTrace,
    /// Commands executed so far
    commands_executed: Vec<CommandOutput>,
    /// References loaded so far
    references_loaded: Vec<LoadedReference>,
}

impl SkillExecutor {
    /// Create a new skill executor.
    pub fn new(
        skill_dir: SkillDirectory,
        config: SkillExecutionConfig,
        provider_config: Option<ProviderConfig>,
    ) -> Self {
        let session_id = config
            .session_id
            .clone()
            .unwrap_or_else(generate_session_id);
        let skill_name = skill_dir.skill_file.frontmatter.name.clone();
        let current_body = skill_dir.skill_file.body.clone();

        Self {
            skill_dir,
            config,
            provider_config,
            current_body,
            trace: SkillTrace::new(&session_id, &skill_name),
            commands_executed: Vec::new(),
            references_loaded: Vec::new(),
        }
    }

    /// Execute the skill and return results.
    pub async fn execute(&mut self) -> RuntimeResult<SkillExecutionResult> {
        // Step 0: Plan
        self.add_trace_step(
            SkillStepKind::Plan,
            &format!(
                "Executing skill '{}' with {} argument chars, {} commands, {} references",
                self.trace.skill_name,
                self.config.arguments.len(),
                self.skill_dir.skill_file.command_executions.len(),
                self.skill_dir.references.len()
            ),
        );

        // Step 1: Substitute $ARGUMENTS
        let arg_count = self.substitute_arguments();
        self.add_trace_step(
            SkillStepKind::ArgumentSubstitution,
            &format!("Substituted {} occurrences of $ARGUMENTS", arg_count),
        );

        // Step 2: Substitute ${CLAUDE_SESSION_ID}
        let session_count = self.substitute_session_id();
        self.add_trace_step(
            SkillStepKind::SessionIdSubstitution,
            &format!(
                "Substituted {} occurrences of ${{CLAUDE_SESSION_ID}}",
                session_count
            ),
        );

        // Step 3: Load reference files
        let ref_count = self.include_references();
        if ref_count > 0 {
            self.add_trace_step(
                SkillStepKind::ReferenceLoading,
                &format!("Loaded {} reference files", ref_count),
            );
        }

        // Step 4: Execute commands
        if self.config.enable_commands {
            self.execute_commands().await?;
        }

        // Step 5: Build final prompt
        let final_prompt = self.build_final_prompt();
        self.add_trace_step(
            SkillStepKind::PromptBuild,
            &format!("Built final prompt ({} chars)", final_prompt.len()),
        );

        // Step 6: Call LLM (if not template_only)
        let llm_response = if self.config.template_only {
            None
        } else if let Some(provider_config) = self.provider_config.clone() {
            self.add_trace_step(
                SkillStepKind::LlmCall,
                &format!(
                    "Calling {} ({})",
                    provider_config.provider, provider_config.model
                ),
            );

            let client = provider::create_client(&provider_config)?;
            let response = client.complete(&final_prompt).await?;
            Some(response)
        } else {
            return Err(RuntimeError::ConfigError(
                "No provider configured and template_only is false".to_string(),
            ));
        };

        // Step 7: Summary
        self.add_trace_step(
            SkillStepKind::Summary,
            &format!(
                "Execution completed: {} commands, {} references",
                self.commands_executed.len(),
                self.references_loaded.len()
            ),
        );

        self.trace.complete(llm_response.clone());

        Ok(SkillExecutionResult {
            trace: self.trace.clone(),
            final_prompt,
            llm_response,
            commands_executed: self.commands_executed.clone(),
            references_loaded: self.references_loaded.clone(),
        })
    }

    /// Substitute all $ARGUMENTS occurrences.
    fn substitute_arguments(&mut self) -> usize {
        let count = self.current_body.matches("$ARGUMENTS").count();
        self.current_body = self
            .current_body
            .replace("$ARGUMENTS", &self.config.arguments);
        count
    }

    /// Substitute all ${CLAUDE_SESSION_ID} occurrences.
    fn substitute_session_id(&mut self) -> usize {
        let pattern = "${CLAUDE_SESSION_ID}";
        let count = self.current_body.matches(pattern).count();
        let session_id = self
            .config
            .session_id
            .clone()
            .unwrap_or_else(|| self.trace.session_id.clone());
        self.current_body = self.current_body.replace(pattern, &session_id);
        count
    }

    /// Load and include referenced files.
    fn include_references(&mut self) -> usize {
        let mut count = 0;
        let mut reference_content = String::new();

        // Find all markdown links that point to reference files
        for link in &self.skill_dir.skill_file.markdown_links {
            // Skip external URLs
            if link.target.starts_with("http://") || link.target.starts_with("https://") {
                continue;
            }

            // Find matching reference file
            if let Some(ref_file) = self.skill_dir.references.iter().find(|r| {
                let ref_path = r.path.to_string_lossy();
                let target = link.target.trim_start_matches("./");
                ref_path == target || ref_path.ends_with(target)
            }) {
                reference_content.push_str(&format!(
                    "\n\n--- {} ---\n{}\n",
                    link.text, ref_file.content
                ));

                self.references_loaded.push(LoadedReference {
                    path: ref_file.path.clone(),
                    line_count: ref_file.line_count,
                    link_text: link.text.clone(),
                });

                count += 1;
            }
        }

        if !reference_content.is_empty() {
            self.current_body.push_str("\n\n## References\n");
            self.current_body.push_str(&reference_content);
        }

        count
    }

    /// Execute all commands in the skill body.
    async fn execute_commands(&mut self) -> RuntimeResult<()> {
        // Clone commands to avoid borrow issues
        let commands: Vec<CommandExecution> = self.skill_dir.skill_file.command_executions.clone();

        for cmd in commands {
            // Check for breakpoints
            if self.should_break_on_command(&cmd.command) {
                self.add_trace_step(
                    SkillStepKind::CommandExecution,
                    &format!("Breakpoint hit: {}", cmd.command),
                );
                // In a real debugger, we'd pause here
                // For now, just log it
            }

            let output = self.execute_single_command(&cmd.command).await?;

            self.add_trace_step(
                SkillStepKind::CommandExecution,
                &format!(
                    "Executed: {} (exit: {}, {}ms)",
                    cmd.command, output.exit_code, output.duration_ms
                ),
            );

            // Replace the `!command` in the body with the output
            let command_pattern = format!("`!{}`", cmd.command);
            self.current_body = self.current_body.replace(&command_pattern, &output.stdout);

            self.commands_executed.push(output);
        }

        Ok(())
    }

    /// Execute a single command.
    async fn execute_single_command(&self, command: &str) -> RuntimeResult<CommandOutput> {
        use tokio::process::Command;
        use tokio::time::timeout;

        let start = std::time::Instant::now();

        let result = timeout(
            Duration::from_millis(self.config.command_timeout_ms),
            Command::new("sh")
                .arg("-c")
                .arg(command)
                .current_dir(&self.config.working_directory)
                .output(),
        )
        .await;

        match result {
            Ok(Ok(output)) => Ok(CommandOutput {
                command: command.to_string(),
                exit_code: output.status.code().unwrap_or(-1),
                stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                duration_ms: start.elapsed().as_millis() as u64,
            }),
            Ok(Err(e)) => Err(RuntimeError::CommandExecutionError {
                command: command.to_string(),
                message: e.to_string(),
            }),
            Err(_) => Err(RuntimeError::CommandTimeout {
                command: command.to_string(),
                timeout_ms: self.config.command_timeout_ms,
            }),
        }
    }

    /// Check if we should break on this command.
    fn should_break_on_command(&self, command: &str) -> bool {
        for bp in &self.config.breakpoints {
            if let Some(pattern) = &bp.command_pattern {
                // Simple glob matching
                if pattern.ends_with('*') {
                    let prefix = &pattern[..pattern.len() - 1];
                    if command.starts_with(prefix) {
                        return true;
                    }
                } else if pattern == command {
                    return true;
                }
            }
        }
        false
    }

    /// Build the final prompt.
    fn build_final_prompt(&self) -> String {
        self.current_body.clone()
    }

    /// Add a trace step.
    fn add_trace_step(&mut self, kind: SkillStepKind, explanation: &str) {
        let step =
            SkillTraceStep::new(self.trace.steps.len() as u32, kind).with_explanation(explanation);
        self.trace.add_step(step);
    }
}

/// Simple skill execution helper.
pub async fn execute_skill(
    skill_path: &std::path::Path,
    arguments: &str,
    provider_config: Option<&ProviderConfig>,
) -> RuntimeResult<SkillExecutionResult> {
    let skill_dir = SkillDirectory::load(skill_path).map_err(|e| match e {
        SkillDirectoryError::MissingSkillFile => {
            RuntimeError::ConfigError("SKILL.md not found in directory".to_string())
        }
        SkillDirectoryError::IoError { path, source } => {
            RuntimeError::ConfigError(format!("Failed to read {}: {}", path.display(), source))
        }
        SkillDirectoryError::ParseError(e) => {
            RuntimeError::ConfigError(format!("Failed to parse SKILL.md: {}", e))
        }
    })?;

    let config = SkillExecutionConfig {
        arguments: arguments.to_string(),
        session_id: Some(generate_session_id()),
        working_directory: skill_path.to_path_buf(),
        enable_commands: true,
        command_timeout_ms: 30000,
        template_only: provider_config.is_none(),
        breakpoints: vec![],
    };

    let mut executor = SkillExecutor::new(skill_dir, config, provider_config.cloned());
    executor.execute().await
}

/// Generate a unique session ID.
fn generate_session_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("skill_{:x}", timestamp)
}

/// Get current timestamp in ISO 8601 format.
fn chrono_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    // Simple ISO-like timestamp without chrono dependency
    let secs = duration.as_secs();
    format!("{}Z", secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_skill() -> (TempDir, SkillDirectory) {
        let dir = TempDir::new().unwrap();

        // Create SKILL.md
        fs::write(
            dir.path().join("SKILL.md"),
            r#"---
name: test-skill
description: A test skill
allowed-tools:
  - Read
---

Search for $ARGUMENTS in the codebase.
Session: ${CLAUDE_SESSION_ID}

See [docs](references/docs.md) for more info.
"#,
        )
        .unwrap();

        // Create references/docs.md
        fs::create_dir(dir.path().join("references")).unwrap();
        fs::write(
            dir.path().join("references/docs.md"),
            "# Documentation\n\nThis is reference content.",
        )
        .unwrap();

        let skill_dir = SkillDirectory::load(dir.path()).unwrap();
        (dir, skill_dir)
    }

    #[test]
    fn test_substitute_arguments() {
        let (_dir, skill_dir) = create_test_skill();
        let config = SkillExecutionConfig {
            arguments: "hello world".to_string(),
            ..Default::default()
        };

        let mut executor = SkillExecutor::new(skill_dir, config, None);
        let count = executor.substitute_arguments();

        assert_eq!(count, 1);
        assert!(executor.current_body.contains("hello world"));
        assert!(!executor.current_body.contains("$ARGUMENTS"));
    }

    #[test]
    fn test_substitute_session_id() {
        let (_dir, skill_dir) = create_test_skill();
        let config = SkillExecutionConfig {
            session_id: Some("test-session-123".to_string()),
            ..Default::default()
        };

        let mut executor = SkillExecutor::new(skill_dir, config, None);
        let count = executor.substitute_session_id();

        assert_eq!(count, 1);
        assert!(executor.current_body.contains("test-session-123"));
        assert!(!executor.current_body.contains("${CLAUDE_SESSION_ID}"));
    }

    #[test]
    fn test_include_references() {
        let (_dir, skill_dir) = create_test_skill();
        let config = SkillExecutionConfig::default();

        let mut executor = SkillExecutor::new(skill_dir, config, None);
        let count = executor.include_references();

        assert_eq!(count, 1);
        assert!(executor.current_body.contains("## References"));
        assert!(executor.current_body.contains("--- docs ---"));
        assert!(executor.current_body.contains("This is reference content"));
        assert_eq!(executor.references_loaded.len(), 1);
        assert_eq!(executor.references_loaded[0].link_text, "docs");
    }

    #[tokio::test]
    async fn test_execute_template_only() {
        let (_dir, skill_dir) = create_test_skill();
        let config = SkillExecutionConfig {
            arguments: "test query".to_string(),
            session_id: Some("sess-test".to_string()),
            template_only: true,
            enable_commands: false,
            ..Default::default()
        };

        let mut executor = SkillExecutor::new(skill_dir, config, None);
        let result = executor.execute().await.unwrap();

        assert!(result.trace.completed);
        assert!(result.llm_response.is_none());
        assert!(result.final_prompt.contains("test query"));
        assert!(result.final_prompt.contains("sess-test"));
        assert_eq!(result.references_loaded.len(), 1);
    }

    #[test]
    fn test_skill_trace_creation() {
        let trace = SkillTrace::new("session-1", "my-skill");
        assert_eq!(trace.session_id, "session-1");
        assert_eq!(trace.skill_name, "my-skill");
        assert!(!trace.completed);
        assert!(trace.steps.is_empty());
    }

    #[test]
    fn test_trace_step_creation() {
        let step =
            SkillTraceStep::new(0, SkillStepKind::Plan).with_explanation("Starting execution");

        assert_eq!(step.index, 0);
        assert_eq!(step.kind, SkillStepKind::Plan);
        assert_eq!(step.explanation, Some("Starting execution".to_string()));
    }

    #[test]
    fn test_command_breakpoint_matching() {
        let config = SkillExecutionConfig {
            breakpoints: vec![
                CommandBreakpoint {
                    command_pattern: Some("gh*".to_string()),
                    line: None,
                },
                CommandBreakpoint {
                    command_pattern: Some("git status".to_string()),
                    line: None,
                },
            ],
            ..Default::default()
        };

        let (_dir, skill_dir) = create_test_skill();
        let executor = SkillExecutor::new(skill_dir, config, None);

        assert!(executor.should_break_on_command("gh pr view"));
        assert!(executor.should_break_on_command("gh issue list"));
        assert!(executor.should_break_on_command("git status"));
        assert!(!executor.should_break_on_command("git diff"));
        assert!(!executor.should_break_on_command("echo hello"));
    }
}

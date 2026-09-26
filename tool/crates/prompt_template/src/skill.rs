//! Claude Code skill file parser.
//!
//! Parses SKILL.md files with YAML frontmatter and extracts:
//! - Frontmatter metadata (name, description, allowed-tools, etc.)
//! - Substitution locations ($ARGUMENTS, ${CLAUDE_SESSION_ID})
//! - Dynamic command executions (`!command`)
//! - Markdown links to referenced files

use prompt_ir::TextRange;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

/// Error type for skill parsing.
#[derive(Error, Debug)]
pub enum SkillError {
    #[error("Missing YAML frontmatter (file must start with ---)")]
    MissingFrontmatter,

    #[error("Unclosed frontmatter (missing closing ---)")]
    UnclosedFrontmatter,

    #[error("Invalid YAML in frontmatter: {0}")]
    InvalidYaml(String),

    #[error("Missing required field: {0}")]
    MissingRequiredField(String),

    #[error("Invalid field value: {field} - {message}")]
    InvalidFieldValue { field: String, message: String },
}

/// Document type detection result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentKind {
    /// Template file (.rtpl) with {{ }} and {% %} syntax
    Template,
    /// Skill file (SKILL.md) with YAML frontmatter
    Skill,
    /// Plain text prompt without special syntax
    PlainPrompt,
}

/// Parsed YAML frontmatter from a skill file.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub struct SkillFrontmatter {
    /// Skill name (required) - becomes the /slash-command
    #[serde(default)]
    pub name: String,

    /// Skill description (required) - used for discovery
    #[serde(default)]
    pub description: String,

    /// If true, only the user can invoke this skill
    #[serde(default)]
    pub disable_model_invocation: bool,

    /// If false, only Claude can invoke this skill
    #[serde(default = "default_user_invocable")]
    pub user_invocable: bool,

    /// Allowed tools when this skill is active
    #[serde(default)]
    pub allowed_tools: Vec<String>,

    /// Specific model to use for this skill
    #[serde(default)]
    pub model: Option<String>,

    /// Context mode ("fork" for isolated subagent)
    #[serde(default)]
    pub context: Option<String>,

    /// Subagent type (Explore, Plan, general-purpose)
    #[serde(default)]
    pub agent: Option<String>,

    /// Autocomplete hint for arguments
    #[serde(default)]
    pub argument_hint: Option<String>,

    /// Additional unknown fields
    #[serde(flatten)]
    pub extra: HashMap<String, serde_yaml::Value>,
}

fn default_user_invocable() -> bool {
    true
}

/// A parsed skill file.
#[derive(Debug, Clone)]
pub struct ParsedSkill {
    /// Parsed frontmatter metadata
    pub frontmatter: SkillFrontmatter,
    /// Source range of the frontmatter block (including --- delimiters)
    pub frontmatter_range: TextRange,
    /// The markdown body after frontmatter
    pub body: String,
    /// Source range of the body
    pub body_range: TextRange,
    /// Locations of $ARGUMENTS substitutions
    pub arguments_locations: Vec<SubstitutionLocation>,
    /// Locations of ${CLAUDE_SESSION_ID} substitutions
    pub session_id_locations: Vec<SubstitutionLocation>,
    /// Dynamic command executions (`!command`)
    pub command_executions: Vec<CommandExecution>,
    /// Markdown links to other files
    pub markdown_links: Vec<MarkdownLink>,
}

/// A substitution location in the skill body.
#[derive(Debug, Clone)]
pub struct SubstitutionLocation {
    /// The full substitution text (e.g., "$ARGUMENTS" or "${CLAUDE_SESSION_ID}")
    pub text: String,
    /// Source range
    pub range: TextRange,
}

/// A dynamic command execution in the skill body.
#[derive(Debug, Clone)]
pub struct CommandExecution {
    /// The command to execute (without the `! backticks)
    pub command: String,
    /// Source range of the entire `!command` block
    pub range: TextRange,
}

/// A markdown link found in the skill body.
#[derive(Debug, Clone)]
pub struct MarkdownLink {
    /// Display text [text]
    pub text: String,
    /// Target path (path/to/file.md)
    pub target: String,
    /// Source range of the entire [text](target)
    pub range: TextRange,
}

/// Detect the document kind based on filename and content.
pub fn detect_document_kind(filename: &str, content: &str) -> DocumentKind {
    let filename_lower = filename.to_lowercase();

    // Check for SKILL.md files
    if filename_lower == "skill.md" || filename_lower.ends_with("/skill.md") {
        // Verify it has frontmatter
        if content.trim_start().starts_with("---") {
            return DocumentKind::Skill;
        }
    }

    // Check for .rtpl template files
    if filename_lower.ends_with(".rtpl") {
        return DocumentKind::Template;
    }

    // Check content for template syntax
    if content.contains("{{") || content.contains("{%") {
        return DocumentKind::Template;
    }

    // Check for YAML frontmatter (could be a skill in a non-standard location)
    if content.trim_start().starts_with("---") {
        // Try to parse as skill frontmatter
        if extract_frontmatter(content).is_ok() {
            return DocumentKind::Skill;
        }
    }

    DocumentKind::PlainPrompt
}

/// Parse a skill file and extract all components.
pub fn parse_skill(content: &str) -> Result<ParsedSkill, SkillError> {
    // Extract frontmatter
    let (frontmatter_str, body, frontmatter_range, body_range) = extract_frontmatter(content)?;

    // Parse YAML frontmatter
    let frontmatter: SkillFrontmatter = serde_yaml::from_str(&frontmatter_str)
        .map_err(|e| SkillError::InvalidYaml(e.to_string()))?;

    // Find substitutions in body
    let arguments_locations = find_substitutions(&body, "$ARGUMENTS", body_range.start_line);
    let session_id_locations =
        find_substitutions(&body, "${CLAUDE_SESSION_ID}", body_range.start_line);

    // Find command executions
    let command_executions = find_command_executions(&body, body_range.start_line);

    // Find markdown links
    let markdown_links = find_markdown_links(&body, body_range.start_line);

    Ok(ParsedSkill {
        frontmatter,
        frontmatter_range,
        body,
        body_range,
        arguments_locations,
        session_id_locations,
        command_executions,
        markdown_links,
    })
}

/// Extract frontmatter from content.
/// Returns (frontmatter_yaml, body, frontmatter_range, body_range)
pub fn extract_frontmatter(
    content: &str,
) -> Result<(String, String, TextRange, TextRange), SkillError> {
    let lines: Vec<&str> = content.lines().collect();

    // Find opening ---
    let trimmed_first = lines.first().map(|l| l.trim()).unwrap_or("");
    if trimmed_first != "---" {
        return Err(SkillError::MissingFrontmatter);
    }

    // Find closing ---
    let mut closing_line = None;
    for (i, line) in lines.iter().enumerate().skip(1) {
        if line.trim() == "---" {
            closing_line = Some(i);
            break;
        }
    }

    let closing_line = closing_line.ok_or(SkillError::UnclosedFrontmatter)?;

    // Extract frontmatter content (between the --- lines)
    let frontmatter_str: String = lines[1..closing_line].join("\n");

    // Extract body (after closing ---)
    let body: String = if closing_line + 1 < lines.len() {
        lines[closing_line + 1..].join("\n")
    } else {
        String::new()
    };

    let frontmatter_range = TextRange::new(0, 0, closing_line as u32, 3);

    let body_start_line = (closing_line + 1) as u32;
    let body_end_line = (lines.len().saturating_sub(1)) as u32;
    let body_end_col = lines.last().map(|l| l.len() as u32).unwrap_or(0);
    let body_range = TextRange::new(body_start_line, 0, body_end_line, body_end_col);

    Ok((frontmatter_str, body, frontmatter_range, body_range))
}

/// Find all occurrences of a substitution pattern in the body.
fn find_substitutions(
    body: &str,
    pattern: &str,
    body_start_line: u32,
) -> Vec<SubstitutionLocation> {
    let mut locations = Vec::new();

    for (line_idx, line) in body.lines().enumerate() {
        let mut search_start = 0;
        while let Some(col) = line[search_start..].find(pattern) {
            let actual_col = search_start + col;
            locations.push(SubstitutionLocation {
                text: pattern.to_string(),
                range: TextRange::new(
                    body_start_line + line_idx as u32,
                    actual_col as u32,
                    body_start_line + line_idx as u32,
                    (actual_col + pattern.len()) as u32,
                ),
            });
            search_start = actual_col + pattern.len();
        }
    }

    locations
}

/// Find all `!command` executions in the body.
fn find_command_executions(body: &str, body_start_line: u32) -> Vec<CommandExecution> {
    let mut executions = Vec::new();

    for (line_idx, line) in body.lines().enumerate() {
        let mut chars = line.chars().peekable();
        let mut col = 0usize;

        while let Some(c) = chars.next() {
            // Look for `! pattern (backtick, bang, then command until closing backtick)
            if c == '`' && chars.peek() == Some(&'!') {
                let start_col = col;
                chars.next(); // consume !
                col += 2;

                // Collect command until closing backtick
                let mut command = String::new();
                let mut found_close = false;

                for c in chars.by_ref() {
                    col += 1;
                    if c == '`' {
                        found_close = true;
                        break;
                    }
                    command.push(c);
                }

                if found_close && !command.is_empty() {
                    executions.push(CommandExecution {
                        command: command.trim().to_string(),
                        range: TextRange::new(
                            body_start_line + line_idx as u32,
                            start_col as u32,
                            body_start_line + line_idx as u32,
                            col as u32,
                        ),
                    });
                }
            } else {
                col += 1;
            }
        }
    }

    executions
}

/// Find all markdown links [text](target) in the body.
fn find_markdown_links(body: &str, body_start_line: u32) -> Vec<MarkdownLink> {
    let mut links = Vec::new();

    for (line_idx, line) in body.lines().enumerate() {
        let mut chars = line.chars().peekable();
        let mut col = 0usize;

        while let Some(c) = chars.next() {
            if c == '[' {
                let start_col = col;

                // Collect text until ]
                let mut text = String::new();
                let mut found_close_bracket = false;

                for c in chars.by_ref() {
                    col += 1;
                    if c == ']' {
                        found_close_bracket = true;
                        break;
                    }
                    text.push(c);
                }

                if !found_close_bracket {
                    col += 1;
                    continue;
                }

                // Check for (
                if chars.peek() != Some(&'(') {
                    col += 1;
                    continue;
                }
                chars.next(); // consume (
                col += 1;

                // Collect target until )
                let mut target = String::new();
                let mut found_close_paren = false;

                for c in chars.by_ref() {
                    col += 1;
                    if c == ')' {
                        found_close_paren = true;
                        break;
                    }
                    target.push(c);
                }

                if found_close_paren && !target.is_empty() {
                    links.push(MarkdownLink {
                        text,
                        target,
                        range: TextRange::new(
                            body_start_line + line_idx as u32,
                            start_col as u32,
                            body_start_line + line_idx as u32,
                            col as u32,
                        ),
                    });
                }
            }
            col += 1;
        }
    }

    links
}

/// Validate skill frontmatter and return diagnostics.
pub fn validate_frontmatter(skill: &ParsedSkill) -> Vec<FrontmatterDiagnostic> {
    let mut diagnostics = Vec::new();
    let fm = &skill.frontmatter;

    // Required: name
    if fm.name.is_empty() {
        diagnostics.push(FrontmatterDiagnostic {
            message: "Missing required field: name".to_string(),
            code: "skill/frontmatter-missing-name".to_string(),
            severity: DiagnosticSeverity::Error,
            range: skill.frontmatter_range,
        });
    } else if fm.name.len() > 64 {
        diagnostics.push(FrontmatterDiagnostic {
            message: format!(
                "Field 'name' exceeds 64 characters ({} chars)",
                fm.name.len()
            ),
            code: "skill/frontmatter-name-too-long".to_string(),
            severity: DiagnosticSeverity::Warning,
            range: skill.frontmatter_range,
        });
    }

    // Required: description
    if fm.description.is_empty() {
        diagnostics.push(FrontmatterDiagnostic {
            message: "Missing required field: description".to_string(),
            code: "skill/frontmatter-missing-description".to_string(),
            severity: DiagnosticSeverity::Error,
            range: skill.frontmatter_range,
        });
    } else if fm.description.len() > 1024 {
        diagnostics.push(FrontmatterDiagnostic {
            message: format!(
                "Field 'description' exceeds 1024 characters ({} chars)",
                fm.description.len()
            ),
            code: "skill/frontmatter-description-too-long".to_string(),
            severity: DiagnosticSeverity::Warning,
            range: skill.frontmatter_range,
        });
    }

    // Validate allowed-tools
    const KNOWN_TOOLS: &[&str] = &[
        "Read",
        "Write",
        "Edit",
        "Bash",
        "Glob",
        "Grep",
        "WebFetch",
        "WebSearch",
        "TodoWrite",
        "Task",
        "Skill",
        "NotebookEdit",
        "AskUserQuestion",
    ];

    for tool in &fm.allowed_tools {
        // Extract base tool name (handle patterns like "Bash(python:*)")
        let base_tool = tool.split('(').next().unwrap_or(tool);

        if !KNOWN_TOOLS.contains(&base_tool) && !tool.starts_with("mcp__") {
            diagnostics.push(FrontmatterDiagnostic {
                message: format!("Unknown tool '{}'. Known tools: {:?}", tool, KNOWN_TOOLS),
                code: "skill/frontmatter-unknown-tool".to_string(),
                severity: DiagnosticSeverity::Warning,
                range: skill.frontmatter_range,
            });
        }
    }

    // Check argument-hint without $ARGUMENTS usage
    if fm.argument_hint.is_some() && skill.arguments_locations.is_empty() {
        diagnostics.push(FrontmatterDiagnostic {
            message: "argument-hint is set but $ARGUMENTS is never used in the skill body"
                .to_string(),
            code: "skill/unused-arguments".to_string(),
            severity: DiagnosticSeverity::Warning,
            range: skill.frontmatter_range,
        });
    }

    // Warn about unknown extra fields
    for key in fm.extra.keys() {
        diagnostics.push(FrontmatterDiagnostic {
            message: format!("Unknown frontmatter field: '{}'", key),
            code: "skill/frontmatter-unknown-field".to_string(),
            severity: DiagnosticSeverity::Info,
            range: skill.frontmatter_range,
        });
    }

    diagnostics
}

/// Diagnostic severity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
}

/// A diagnostic for frontmatter validation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FrontmatterDiagnostic {
    pub message: String,
    pub code: String,
    pub severity: DiagnosticSeverity,
    pub range: TextRange,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_skill_file() {
        let content = r#"---
name: test-skill
description: A test skill
---
Hello $ARGUMENTS"#;

        assert_eq!(
            detect_document_kind("SKILL.md", content),
            DocumentKind::Skill
        );
        assert_eq!(
            detect_document_kind("my-skill/SKILL.md", content),
            DocumentKind::Skill
        );
    }

    #[test]
    fn test_detect_template_file() {
        let content = "Hello {{ name }}!";
        assert_eq!(
            detect_document_kind("prompt.rtpl", content),
            DocumentKind::Template
        );
        assert_eq!(
            detect_document_kind("test.txt", content),
            DocumentKind::Template
        );
    }

    #[test]
    fn test_detect_plain_prompt() {
        let content = "You are a helpful assistant.";
        assert_eq!(
            detect_document_kind("prompt.txt", content),
            DocumentKind::PlainPrompt
        );
    }

    #[test]
    fn test_parse_skill_frontmatter() {
        let content = r#"---
name: code-review
description: Review code for quality issues
allowed-tools:
  - Read
  - Grep
  - Glob
argument-hint: "[file-or-pr]"
---

Review the code at $ARGUMENTS."#;

        let skill = parse_skill(content).unwrap();
        assert_eq!(skill.frontmatter.name, "code-review");
        assert_eq!(
            skill.frontmatter.description,
            "Review code for quality issues"
        );
        assert_eq!(
            skill.frontmatter.allowed_tools,
            vec!["Read", "Grep", "Glob"]
        );
        assert_eq!(
            skill.frontmatter.argument_hint,
            Some("[file-or-pr]".to_string())
        );
    }

    #[test]
    fn test_parse_skill_substitutions() {
        let content = r#"---
name: test
description: Test skill
---

Search for $ARGUMENTS in the codebase.
Session: ${CLAUDE_SESSION_ID}
Again: $ARGUMENTS"#;

        let skill = parse_skill(content).unwrap();
        assert_eq!(skill.arguments_locations.len(), 2);
        assert_eq!(skill.session_id_locations.len(), 1);
    }

    #[test]
    fn test_parse_skill_command_executions() {
        let content = r#"---
name: pr-review
description: Review a PR
---

Current diff: `!gh pr diff`
Comments: `!gh pr view --comments`"#;

        let skill = parse_skill(content).unwrap();
        assert_eq!(skill.command_executions.len(), 2);
        assert_eq!(skill.command_executions[0].command, "gh pr diff");
        assert_eq!(skill.command_executions[1].command, "gh pr view --comments");
    }

    #[test]
    fn test_parse_skill_markdown_links() {
        let content = r#"---
name: test
description: Test skill
---

See [API docs](references/api.md) for details.
Also check [examples](resources/examples/sample.md)."#;

        let skill = parse_skill(content).unwrap();
        assert_eq!(skill.markdown_links.len(), 2);
        assert_eq!(skill.markdown_links[0].text, "API docs");
        assert_eq!(skill.markdown_links[0].target, "references/api.md");
        assert_eq!(skill.markdown_links[1].text, "examples");
        assert_eq!(
            skill.markdown_links[1].target,
            "resources/examples/sample.md"
        );
    }

    #[test]
    fn test_validate_missing_name() {
        let content = r#"---
description: A skill without a name
---
Body"#;

        let skill = parse_skill(content).unwrap();
        let diagnostics = validate_frontmatter(&skill);
        assert!(diagnostics
            .iter()
            .any(|d| d.code == "skill/frontmatter-missing-name"));
    }

    #[test]
    fn test_validate_missing_description() {
        let content = r#"---
name: test-skill
---
Body"#;

        let skill = parse_skill(content).unwrap();
        let diagnostics = validate_frontmatter(&skill);
        assert!(diagnostics
            .iter()
            .any(|d| d.code == "skill/frontmatter-missing-description"));
    }

    #[test]
    fn test_validate_unknown_tool() {
        let content = r#"---
name: test-skill
description: Test
allowed-tools:
  - Read
  - UnknownTool
---
Body"#;

        let skill = parse_skill(content).unwrap();
        let diagnostics = validate_frontmatter(&skill);
        assert!(diagnostics
            .iter()
            .any(|d| d.code == "skill/frontmatter-unknown-tool"));
    }

    #[test]
    fn test_validate_unused_arguments() {
        let content = r#"---
name: test-skill
description: Test
argument-hint: "[query]"
---
This skill does not use the arguments."#;

        let skill = parse_skill(content).unwrap();
        let diagnostics = validate_frontmatter(&skill);
        assert!(diagnostics
            .iter()
            .any(|d| d.code == "skill/unused-arguments"));
    }

    #[test]
    fn test_missing_frontmatter_error() {
        let content = "No frontmatter here";
        let result = parse_skill(content);
        assert!(matches!(result, Err(SkillError::MissingFrontmatter)));
    }

    #[test]
    fn test_unclosed_frontmatter_error() {
        let content = r#"---
name: test
description: Test
No closing delimiter"#;

        let result = parse_skill(content);
        assert!(matches!(result, Err(SkillError::UnclosedFrontmatter)));
    }

    #[test]
    fn test_bash_tool_with_pattern() {
        let content = r#"---
name: test-skill
description: Test
allowed-tools:
  - Bash(python:*)
  - Bash(gh:*)
---
Body"#;

        let skill = parse_skill(content).unwrap();
        let diagnostics = validate_frontmatter(&skill);
        // Should not have unknown tool warnings for Bash patterns
        assert!(!diagnostics
            .iter()
            .any(|d| d.code == "skill/frontmatter-unknown-tool"));
    }
}

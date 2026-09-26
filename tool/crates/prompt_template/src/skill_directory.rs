//! Skill directory validation.
//!
//! Validates skill directories with their supporting files:
//! - references/ - Reference documentation loaded on-demand
//! - scripts/ - Executable scripts
//! - resources/ - Examples and templates
//! - assets/ - Output templates (not loaded into context)

use crate::skill::{parse_skill, DiagnosticSeverity, MarkdownLink, ParsedSkill, SkillError};
use prompt_ir::TextRange;
use std::path::{Path, PathBuf};
use std::{fs, io};
use thiserror::Error;

/// Error type for skill directory operations.
#[derive(Error, Debug)]
pub enum SkillDirectoryError {
    #[error("SKILL.md not found in directory")]
    MissingSkillFile,

    #[error("Failed to read file {path}: {source}")]
    IoError { path: PathBuf, source: io::Error },

    #[error("Failed to parse SKILL.md: {0}")]
    ParseError(#[from] SkillError),
}

/// A loaded skill directory with all its components.
#[derive(Debug)]
pub struct SkillDirectory {
    /// Root directory of the skill
    pub root: PathBuf,
    /// Parsed SKILL.md file
    pub skill_file: ParsedSkill,
    /// Reference files (references/*.md and top-level *.md)
    pub references: Vec<ReferenceFile>,
    /// Script files (scripts/*)
    pub scripts: Vec<ScriptFile>,
    /// Resource files (resources/*)
    pub resources: Vec<ResourceFile>,
}

/// A reference file in the skill directory.
#[derive(Debug)]
pub struct ReferenceFile {
    /// Path relative to skill root
    pub path: PathBuf,
    /// File content
    pub content: String,
    /// Number of lines
    pub line_count: usize,
    /// Links from SKILL.md that reference this file
    pub referenced_from: Vec<MarkdownLink>,
}

/// A script file in the skill directory.
#[derive(Debug)]
pub struct ScriptFile {
    /// Path relative to skill root
    pub path: PathBuf,
    /// File content
    pub content: String,
    /// Links from SKILL.md that reference this file
    pub referenced_from: Vec<MarkdownLink>,
}

/// A resource file in the skill directory.
#[derive(Debug)]
pub struct ResourceFile {
    /// Path relative to skill root
    pub path: PathBuf,
    /// Links from SKILL.md that reference this file
    pub referenced_from: Vec<MarkdownLink>,
}

/// A diagnostic for skill directory validation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DirectoryDiagnostic {
    pub message: String,
    pub code: String,
    pub severity: DiagnosticSeverity,
    /// Range in SKILL.md (if applicable)
    pub range: Option<TextRange>,
    /// Path to the file this diagnostic refers to (if applicable)
    pub file_path: Option<PathBuf>,
}

impl SkillDirectory {
    /// Load a skill directory from the given path.
    pub fn load(dir: &Path) -> Result<Self, SkillDirectoryError> {
        let skill_path = dir.join("SKILL.md");
        if !skill_path.exists() {
            return Err(SkillDirectoryError::MissingSkillFile);
        }

        // Read and parse SKILL.md
        let skill_content =
            fs::read_to_string(&skill_path).map_err(|e| SkillDirectoryError::IoError {
                path: skill_path.clone(),
                source: e,
            })?;
        let skill_file = parse_skill(&skill_content)?;

        // Scan for references (*.md files in root and references/)
        let mut references = Vec::new();
        scan_references(dir, &skill_file.markdown_links, &mut references)?;

        // Scan for scripts (scripts/*)
        let mut scripts = Vec::new();
        scan_scripts(dir, &skill_file.markdown_links, &mut scripts)?;

        // Scan for resources (resources/*)
        let mut resources = Vec::new();
        scan_resources(dir, &skill_file.markdown_links, &mut resources)?;

        Ok(SkillDirectory {
            root: dir.to_path_buf(),
            skill_file,
            references,
            scripts,
            resources,
        })
    }

    /// Validate the skill directory and return diagnostics.
    pub fn validate(&self) -> Vec<DirectoryDiagnostic> {
        let mut diagnostics = Vec::new();

        // Validate references
        self.validate_references(&mut diagnostics);

        // Validate scripts
        self.validate_scripts(&mut diagnostics);

        // Check for orphaned files
        self.check_orphaned_files(&mut diagnostics);

        // Check skill size
        self.check_skill_size(&mut diagnostics);

        diagnostics
    }

    fn validate_references(&self, diagnostics: &mut Vec<DirectoryDiagnostic>) {
        // Check for broken reference links
        for link in &self.skill_file.markdown_links {
            // Skip external URLs
            if link.target.starts_with("http://") || link.target.starts_with("https://") {
                continue;
            }

            let target_path = self.root.join(&link.target);
            if !target_path.exists() {
                diagnostics.push(DirectoryDiagnostic {
                    message: format!("Broken reference: '{}' not found", link.target),
                    code: "skill/broken-reference".to_string(),
                    severity: DiagnosticSeverity::Error,
                    range: Some(link.range),
                    file_path: None,
                });
            }
        }

        // Check reference file sizes
        for reference in &self.references {
            if reference.line_count > 600 {
                diagnostics.push(DirectoryDiagnostic {
                    message: format!(
                        "Reference file '{}' is large ({} lines). Consider splitting into smaller files.",
                        reference.path.display(),
                        reference.line_count
                    ),
                    code: "skill/reference-too-large".to_string(),
                    severity: DiagnosticSeverity::Warning,
                    range: None,
                    file_path: Some(reference.path.clone()),
                });
            }
        }
    }

    fn validate_scripts(&self, diagnostics: &mut Vec<DirectoryDiagnostic>) {
        for script in &self.scripts {
            // Optional: Validate script syntax
            if let Some(ext) = script.path.extension().and_then(|e| e.to_str()) {
                match ext {
                    "py" => {
                        if let Some(diag) = validate_python_syntax(&script.path, &script.content) {
                            diagnostics.push(diag);
                        }
                    }
                    "sh" | "bash" => {
                        if let Some(diag) = validate_bash_syntax(&script.path, &script.content) {
                            diagnostics.push(diag);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn check_orphaned_files(&self, diagnostics: &mut Vec<DirectoryDiagnostic>) {
        // Check references
        for reference in &self.references {
            if reference.referenced_from.is_empty() {
                diagnostics.push(DirectoryDiagnostic {
                    message: format!(
                        "Orphaned file: '{}' is not referenced in SKILL.md",
                        reference.path.display()
                    ),
                    code: "skill/orphaned-file".to_string(),
                    severity: DiagnosticSeverity::Warning,
                    range: None,
                    file_path: Some(reference.path.clone()),
                });
            }
        }

        // Check scripts (only warn, scripts might be utility)
        for script in &self.scripts {
            if script.referenced_from.is_empty() {
                diagnostics.push(DirectoryDiagnostic {
                    message: format!(
                        "Script '{}' is not referenced in SKILL.md (may be a utility script)",
                        script.path.display()
                    ),
                    code: "skill/orphaned-file".to_string(),
                    severity: DiagnosticSeverity::Info,
                    range: None,
                    file_path: Some(script.path.clone()),
                });
            }
        }
    }

    fn check_skill_size(&self, diagnostics: &mut Vec<DirectoryDiagnostic>) {
        let skill_lines = self.skill_file.body.lines().count();
        if skill_lines > 500 {
            diagnostics.push(DirectoryDiagnostic {
                message: format!(
                    "SKILL.md body is large ({} lines). Consider using references/ for detailed content.",
                    skill_lines
                ),
                code: "skill/skill-too-large".to_string(),
                severity: DiagnosticSeverity::Warning,
                range: Some(self.skill_file.body_range),
                file_path: None,
            });
        }
    }

    /// Get total line count across all reference files.
    pub fn total_reference_lines(&self) -> usize {
        self.references.iter().map(|r| r.line_count).sum()
    }
}

/// Scan for reference files in the skill directory.
fn scan_references(
    dir: &Path,
    links: &[MarkdownLink],
    references: &mut Vec<ReferenceFile>,
) -> Result<(), SkillDirectoryError> {
    // Scan top-level .md files (except SKILL.md)
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext == "md" && path.file_name() != Some("SKILL.md".as_ref()) {
                        add_reference_file(dir, &path, links, references)?;
                    }
                }
            }
        }
    }

    // Scan references/ directory
    let references_dir = dir.join("references");
    if references_dir.exists() {
        scan_directory_recursive(&references_dir, dir, links, references)?;
    }

    Ok(())
}

fn scan_directory_recursive(
    scan_dir: &Path,
    root: &Path,
    links: &[MarkdownLink],
    references: &mut Vec<ReferenceFile>,
) -> Result<(), SkillDirectoryError> {
    if let Ok(entries) = fs::read_dir(scan_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                scan_directory_recursive(&path, root, links, references)?;
            } else if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext == "md" {
                        add_reference_file(root, &path, links, references)?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn add_reference_file(
    root: &Path,
    path: &Path,
    links: &[MarkdownLink],
    references: &mut Vec<ReferenceFile>,
) -> Result<(), SkillDirectoryError> {
    let content = fs::read_to_string(path).map_err(|e| SkillDirectoryError::IoError {
        path: path.to_path_buf(),
        source: e,
    })?;

    let relative_path = path.strip_prefix(root).unwrap_or(path);
    let relative_str = relative_path.to_string_lossy();

    // Find links that reference this file
    let referenced_from: Vec<MarkdownLink> = links
        .iter()
        .filter(|link| {
            let target = link.target.trim_start_matches("./");
            target == relative_str || target == relative_path.to_string_lossy()
        })
        .cloned()
        .collect();

    references.push(ReferenceFile {
        path: relative_path.to_path_buf(),
        line_count: content.lines().count(),
        content,
        referenced_from,
    });

    Ok(())
}

/// Scan for script files in the skill directory.
fn scan_scripts(
    dir: &Path,
    links: &[MarkdownLink],
    scripts: &mut Vec<ScriptFile>,
) -> Result<(), SkillDirectoryError> {
    let scripts_dir = dir.join("scripts");
    if !scripts_dir.exists() {
        return Ok(());
    }

    scan_scripts_recursive(&scripts_dir, dir, links, scripts)
}

fn scan_scripts_recursive(
    scan_dir: &Path,
    root: &Path,
    links: &[MarkdownLink],
    scripts: &mut Vec<ScriptFile>,
) -> Result<(), SkillDirectoryError> {
    if let Ok(entries) = fs::read_dir(scan_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                scan_scripts_recursive(&path, root, links, scripts)?;
            } else if path.is_file() {
                let content =
                    fs::read_to_string(&path).map_err(|e| SkillDirectoryError::IoError {
                        path: path.clone(),
                        source: e,
                    })?;

                let relative_path = path.strip_prefix(root).unwrap_or(&path);
                let relative_str = relative_path.to_string_lossy();

                let referenced_from: Vec<MarkdownLink> = links
                    .iter()
                    .filter(|link| {
                        let target = link.target.trim_start_matches("./");
                        target == relative_str || target == relative_path.to_string_lossy()
                    })
                    .cloned()
                    .collect();

                scripts.push(ScriptFile {
                    path: relative_path.to_path_buf(),
                    content,
                    referenced_from,
                });
            }
        }
    }
    Ok(())
}

/// Scan for resource files in the skill directory.
fn scan_resources(
    dir: &Path,
    links: &[MarkdownLink],
    resources: &mut Vec<ResourceFile>,
) -> Result<(), SkillDirectoryError> {
    let resources_dir = dir.join("resources");
    if !resources_dir.exists() {
        return Ok(());
    }

    scan_resources_recursive(&resources_dir, dir, links, resources)
}

fn scan_resources_recursive(
    scan_dir: &Path,
    root: &Path,
    links: &[MarkdownLink],
    resources: &mut Vec<ResourceFile>,
) -> Result<(), SkillDirectoryError> {
    if let Ok(entries) = fs::read_dir(scan_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                scan_resources_recursive(&path, root, links, resources)?;
            } else if path.is_file() {
                let relative_path = path.strip_prefix(root).unwrap_or(&path);
                let relative_str = relative_path.to_string_lossy();

                let referenced_from: Vec<MarkdownLink> = links
                    .iter()
                    .filter(|link| {
                        let target = link.target.trim_start_matches("./");
                        target == relative_str || target == relative_path.to_string_lossy()
                    })
                    .cloned()
                    .collect();

                resources.push(ResourceFile {
                    path: relative_path.to_path_buf(),
                    referenced_from,
                });
            }
        }
    }
    Ok(())
}

/// Validate Python script syntax (basic check).
fn validate_python_syntax(path: &Path, content: &str) -> Option<DirectoryDiagnostic> {
    // Basic syntax check: look for common issues
    // A real implementation would use `python -m py_compile`

    // Check for obvious syntax errors like mismatched quotes
    let mut in_string = false;
    let mut string_char = ' ';
    let mut paren_depth = 0;
    let mut bracket_depth = 0;
    let mut brace_depth = 0;

    for (line_num, line) in content.lines().enumerate() {
        // Skip comments
        let code = if let Some(pos) = line.find('#') {
            if !in_string {
                &line[..pos]
            } else {
                line
            }
        } else {
            line
        };

        for c in code.chars() {
            if in_string {
                if c == string_char {
                    in_string = false;
                }
            } else {
                match c {
                    '"' | '\'' => {
                        in_string = true;
                        string_char = c;
                    }
                    '(' => paren_depth += 1,
                    ')' => {
                        if paren_depth == 0 {
                            return Some(DirectoryDiagnostic {
                                message: format!(
                                    "Syntax error at line {}: unmatched closing parenthesis",
                                    line_num + 1
                                ),
                                code: "skill/script-syntax-error".to_string(),
                                severity: DiagnosticSeverity::Error,
                                range: None,
                                file_path: Some(path.to_path_buf()),
                            });
                        }
                        paren_depth -= 1;
                    }
                    '[' => bracket_depth += 1,
                    ']' => {
                        if bracket_depth == 0 {
                            return Some(DirectoryDiagnostic {
                                message: format!(
                                    "Syntax error at line {}: unmatched closing bracket",
                                    line_num + 1
                                ),
                                code: "skill/script-syntax-error".to_string(),
                                severity: DiagnosticSeverity::Error,
                                range: None,
                                file_path: Some(path.to_path_buf()),
                            });
                        }
                        bracket_depth -= 1;
                    }
                    '{' => brace_depth += 1,
                    '}' => {
                        if brace_depth == 0 {
                            return Some(DirectoryDiagnostic {
                                message: format!(
                                    "Syntax error at line {}: unmatched closing brace",
                                    line_num + 1
                                ),
                                code: "skill/script-syntax-error".to_string(),
                                severity: DiagnosticSeverity::Error,
                                range: None,
                                file_path: Some(path.to_path_buf()),
                            });
                        }
                        brace_depth -= 1;
                    }
                    _ => {}
                }
            }
        }
    }

    if paren_depth != 0 {
        return Some(DirectoryDiagnostic {
            message: "Syntax error: unclosed parenthesis".to_string(),
            code: "skill/script-syntax-error".to_string(),
            severity: DiagnosticSeverity::Error,
            range: None,
            file_path: Some(path.to_path_buf()),
        });
    }
    if bracket_depth != 0 {
        return Some(DirectoryDiagnostic {
            message: "Syntax error: unclosed bracket".to_string(),
            code: "skill/script-syntax-error".to_string(),
            severity: DiagnosticSeverity::Error,
            range: None,
            file_path: Some(path.to_path_buf()),
        });
    }
    if brace_depth != 0 {
        return Some(DirectoryDiagnostic {
            message: "Syntax error: unclosed brace".to_string(),
            code: "skill/script-syntax-error".to_string(),
            severity: DiagnosticSeverity::Error,
            range: None,
            file_path: Some(path.to_path_buf()),
        });
    }

    None
}

/// Validate Bash script syntax (basic check).
fn validate_bash_syntax(path: &Path, content: &str) -> Option<DirectoryDiagnostic> {
    // Basic syntax check for bash scripts
    // A real implementation would use `bash -n`

    let mut if_depth = 0;
    let mut for_depth = 0;
    let mut while_depth = 0;
    let mut case_depth = 0;

    for (line_num, line) in content.lines().enumerate() {
        let trimmed = line.trim();

        // Skip comments and empty lines
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Count control structures
        if trimmed.starts_with("if ") || trimmed == "if" {
            if_depth += 1;
        }
        if trimmed == "fi" || trimmed.ends_with("; fi") {
            if if_depth == 0 {
                return Some(DirectoryDiagnostic {
                    message: format!("Syntax error at line {}: unexpected 'fi'", line_num + 1),
                    code: "skill/script-syntax-error".to_string(),
                    severity: DiagnosticSeverity::Error,
                    range: None,
                    file_path: Some(path.to_path_buf()),
                });
            }
            if_depth -= 1;
        }

        if trimmed.starts_with("for ") {
            for_depth += 1;
        }
        if trimmed == "done" || trimmed.ends_with("; done") {
            if for_depth > 0 {
                for_depth -= 1;
            } else if while_depth > 0 {
                while_depth -= 1;
            }
        }

        if trimmed.starts_with("while ") || trimmed.starts_with("until ") {
            while_depth += 1;
        }

        if trimmed.starts_with("case ") {
            case_depth += 1;
        }
        if trimmed == "esac" {
            if case_depth == 0 {
                return Some(DirectoryDiagnostic {
                    message: format!("Syntax error at line {}: unexpected 'esac'", line_num + 1),
                    code: "skill/script-syntax-error".to_string(),
                    severity: DiagnosticSeverity::Error,
                    range: None,
                    file_path: Some(path.to_path_buf()),
                });
            }
            case_depth -= 1;
        }
    }

    if if_depth != 0 {
        return Some(DirectoryDiagnostic {
            message: "Syntax error: unclosed 'if' block (missing 'fi')".to_string(),
            code: "skill/script-syntax-error".to_string(),
            severity: DiagnosticSeverity::Error,
            range: None,
            file_path: Some(path.to_path_buf()),
        });
    }
    if for_depth != 0 || while_depth != 0 {
        return Some(DirectoryDiagnostic {
            message: "Syntax error: unclosed loop (missing 'done')".to_string(),
            code: "skill/script-syntax-error".to_string(),
            severity: DiagnosticSeverity::Error,
            range: None,
            file_path: Some(path.to_path_buf()),
        });
    }
    if case_depth != 0 {
        return Some(DirectoryDiagnostic {
            message: "Syntax error: unclosed 'case' block (missing 'esac')".to_string(),
            code: "skill/script-syntax-error".to_string(),
            severity: DiagnosticSeverity::Error,
            range: None,
            file_path: Some(path.to_path_buf()),
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_skill_dir() -> TempDir {
        let dir = TempDir::new().unwrap();

        // Create SKILL.md
        fs::write(
            dir.path().join("SKILL.md"),
            r#"---
name: test-skill
description: A test skill for validation
allowed-tools:
  - Read
  - Grep
argument-hint: "[query]"
---

Search for $ARGUMENTS in the codebase.

See [API docs](references/api.md) for details.
Run helper: `!python scripts/helper.py`
"#,
        )
        .unwrap();

        // Create references/
        fs::create_dir(dir.path().join("references")).unwrap();
        fs::write(
            dir.path().join("references/api.md"),
            "# API Documentation\n\nThis is the API docs.",
        )
        .unwrap();

        // Create scripts/
        fs::create_dir(dir.path().join("scripts")).unwrap();
        fs::write(
            dir.path().join("scripts/helper.py"),
            "#!/usr/bin/env python3\nprint('Hello')\n",
        )
        .unwrap();

        dir
    }

    #[test]
    fn test_load_skill_directory() {
        let dir = create_test_skill_dir();
        let skill_dir = SkillDirectory::load(dir.path()).unwrap();

        assert_eq!(skill_dir.skill_file.frontmatter.name, "test-skill");
        assert_eq!(skill_dir.references.len(), 1);
        assert_eq!(skill_dir.scripts.len(), 1);
    }

    #[test]
    fn test_validate_skill_directory() {
        let dir = create_test_skill_dir();
        let skill_dir = SkillDirectory::load(dir.path()).unwrap();
        let diagnostics = skill_dir.validate();

        // Should have no errors (everything is referenced correctly)
        let errors: Vec<_> = diagnostics
            .iter()
            .filter(|d| matches!(d.severity, DiagnosticSeverity::Error))
            .collect();
        assert!(errors.is_empty(), "Unexpected errors: {:?}", errors);
    }

    #[test]
    fn test_broken_reference_detection() {
        let dir = TempDir::new().unwrap();

        fs::write(
            dir.path().join("SKILL.md"),
            r#"---
name: test
description: Test
---

See [broken link](nonexistent.md) for details.
"#,
        )
        .unwrap();

        let skill_dir = SkillDirectory::load(dir.path()).unwrap();
        let diagnostics = skill_dir.validate();

        assert!(diagnostics
            .iter()
            .any(|d| d.code == "skill/broken-reference"));
    }

    #[test]
    fn test_orphaned_file_detection() {
        let dir = TempDir::new().unwrap();

        fs::write(
            dir.path().join("SKILL.md"),
            r#"---
name: test
description: Test
---

No references here.
"#,
        )
        .unwrap();

        // Create an unreferenced file
        fs::write(
            dir.path().join("orphaned.md"),
            "# Orphaned\n\nNot referenced.",
        )
        .unwrap();

        let skill_dir = SkillDirectory::load(dir.path()).unwrap();
        let diagnostics = skill_dir.validate();

        assert!(diagnostics.iter().any(|d| d.code == "skill/orphaned-file"));
    }

    #[test]
    fn test_missing_skill_file() {
        let dir = TempDir::new().unwrap();
        let result = SkillDirectory::load(dir.path());
        assert!(matches!(result, Err(SkillDirectoryError::MissingSkillFile)));
    }

    #[test]
    fn test_python_syntax_validation() {
        let path = PathBuf::from("test.py");

        // Valid Python
        let valid = "def foo():\n    return 42\n";
        assert!(validate_python_syntax(&path, valid).is_none());

        // Unclosed parenthesis
        let invalid = "def foo(:\n    return 42\n";
        assert!(validate_python_syntax(&path, invalid).is_some());
    }

    #[test]
    fn test_bash_syntax_validation() {
        let path = PathBuf::from("test.sh");

        // Valid Bash
        let valid = "if [ -f file ]; then\n    echo ok\nfi\n";
        assert!(validate_bash_syntax(&path, valid).is_none());

        // Unclosed if
        let invalid = "if [ -f file ]; then\n    echo ok\n";
        assert!(validate_bash_syntax(&path, invalid).is_some());
    }
}

//! Template parser and evaluator for PromptDbg.
//!
//! Supports:
//! - Variable interpolation: `{{ variable }}`
//! - Conditionals: `{% if condition %} ... {% endif %}`
//! - Loops: `{% for item in items %} ... {% endfor %}`
//! - Annotations: `// @rule RULE:NAME`
//! - Claude Code skill files (SKILL.md with YAML frontmatter)

pub mod contextgen;
pub mod skill;
pub mod skill_directory;

pub use contextgen::generate_contexts;

use std::collections::HashMap;

use prompt_ir::{IRKind, IRMeta, IRNode, IRNodeMeta, PromptDiagnostic, PromptIR, TextRange};
use serde_json::Value;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TemplateError {
    #[error("Parse error at line {line}: {message}")]
    ParseError { line: u32, message: String },

    #[error("Missing variable: {0}")]
    MissingVariable(String),

    #[error("Invalid expression: {0}")]
    InvalidExpression(String),

    #[error("Unclosed block: {0}")]
    UnclosedBlock(String),
}

impl TemplateError {
    pub fn to_diagnostic(&self, range: TextRange) -> PromptDiagnostic {
        PromptDiagnostic::error(self.to_string(), range)
    }
}

/// Result of template rendering.
#[derive(Debug, Clone)]
pub struct RenderResult {
    /// The rendered output
    pub output: String,
    /// Variables that were used
    pub variables_used: Vec<String>,
    /// Branches that were taken (for debugging)
    pub branches_taken: Vec<BranchInfo>,
    /// Any warnings during rendering
    pub warnings: Vec<String>,
    /// Detailed variable access log
    pub variable_accesses: Vec<VariableAccess>,
    /// Loop iterations performed
    pub loop_iterations: Vec<LoopInfo>,
    /// Absolute source line numbers (0-based) that were emitted, i.e. fell inside
    /// a rendered region. A line inside a non-taken branch is absent. This is the
    /// structural-coverage signal: an annotation node is structurally covered iff
    /// its line appears here. Sorted ascending; loop bodies appear once.
    pub rendered_lines: Vec<u32>,
}

/// Information about a branch taken during rendering.
#[derive(Debug, Clone)]
pub struct BranchInfo {
    /// The condition expression
    pub condition: String,
    /// Whether this branch was taken
    pub taken: bool,
    /// Source range of the entire if/else block
    pub range: TextRange,
    /// Line number where the condition is defined
    pub condition_line: u32,
    /// The evaluated value of the condition
    pub evaluated_value: Option<String>,
    /// Which branch was executed: "if", "else", or "none"
    pub branch_executed: String,
}

/// Information about a variable access.
#[derive(Debug, Clone)]
pub struct VariableAccess {
    /// Variable name/path
    pub name: String,
    /// The value at access time
    pub value: String,
    /// The type of the value
    pub value_type: String,
    /// Line number where accessed
    pub line: u32,
    /// Column where accessed
    pub column: u32,
}

/// Information about a loop iteration.
#[derive(Debug, Clone)]
pub struct LoopInfo {
    /// Loop variable name
    pub variable: String,
    /// Collection being iterated
    pub collection: String,
    /// Number of iterations
    pub iteration_count: usize,
    /// Source range of the for block
    pub range: TextRange,
}

/// Parsed annotation from a prompt file.
#[derive(Debug, Clone)]
pub struct Annotation {
    pub kind: AnnotationKind,
    pub id: String,
    pub attributes: HashMap<String, String>,
    pub range: TextRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnnotationKind {
    Rule,
    Condition,
    Breakpoint,
    Provider,
}

/// Internal rendering context for tracking state.
struct RenderContext<'a> {
    context: &'a Value,
    variables_used: Vec<String>,
    branches_taken: Vec<BranchInfo>,
    variable_accesses: Vec<VariableAccess>,
    loop_iterations: Vec<LoopInfo>,
    warnings: Vec<String>,
    rendered_lines: Vec<u32>,
    current_line: u32,
    /// Absolute source line of the first line of the slice currently being
    /// rendered. Control blocks recurse on substrings, so this offset is what
    /// keeps recorded line numbers absolute (true to the original file) at any
    /// nesting depth, rather than relative to the enclosing block.
    line_offset: u32,
}

impl<'a> RenderContext<'a> {
    fn new(context: &'a Value) -> Self {
        Self {
            context,
            variables_used: Vec::new(),
            branches_taken: Vec::new(),
            variable_accesses: Vec::new(),
            loop_iterations: Vec::new(),
            warnings: Vec::new(),
            rendered_lines: Vec::new(),
            current_line: 0,
            line_offset: 0,
        }
    }

    fn record_variable_access(&mut self, name: &str, value: &Value, line: u32, column: u32) {
        self.variable_accesses.push(VariableAccess {
            name: name.to_string(),
            value: format_value(value),
            value_type: value_type(value),
            line,
            column,
        });
    }
}

/// Format a JSON value for display.
fn format_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
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

// ---------------------------------------------------------------------------
// Control-tag recognition
//
// This is the *single* definition of what counts as a control tag. The renderer
// (`render_internal`, `process_if_block`, `process_for_block`) and the static
// guard-chain analysis ([`guard_chains`]) both go through these functions, so
// the two cannot disagree about where a branch begins or ends. Before this was
// shared, the renderer recognised `{% else %}` anywhere on a line while the
// analyser recognised it only when it was alone on the line, which made the
// analyser report a rule as unreachable in a template where the renderer in fact
// rendered it (a false positive, and a hole in the analyser's soundness claim).
//
// Two distinct rules are involved, and keeping them distinct is deliberate
// because it is what the renderer has always done:
//
//   * [`is_control_line`] is the *dispatch* rule. A line is handed to control
//     handling only when its trimmed form starts with `{%`, so a tag embedded in
//     prose (`* {% if x %}`) is ordinary text and opens nothing.
//   * `opens_if` / `is_else` / `closes_if` / `opens_for` / `closes_for` are the
//     *scanning* rules used to find the end of an already-open block. They match
//     a tag anywhere on the line, so an embedded tag does act as a boundary once
//     a block is open.
//
// Embedded control tags are therefore legal, and the analyser is made to see
// exactly what the renderer sees rather than the renderer being tightened to the
// analyser's stricter reading. [`embedded_control_tags`] exposes that stricter
// reading (a control tag must be alone on its line) for callers that would
// rather reject ambiguous templates up front.

/// True when the renderer dispatches this line to control-flow handling: its
/// trimmed form starts with `{%`.
pub fn is_control_line(line: &str) -> bool {
    line.trim().starts_with("{%")
}

/// The inside of a `{% ... %}` tag occupying a whole line, exactly as
/// `process_control_block` parses it. Deliberately tolerant: it does not reject a
/// second tag later on the same line, because the renderer does not either.
fn control_inner(line: &str) -> Option<&str> {
    line.trim()
        .strip_prefix("{%")
        .and_then(|s| s.strip_suffix("%}"))
        .map(str::trim)
}

fn opens_if(line: &str) -> bool {
    line.contains("{% if ")
}

fn is_else(line: &str) -> bool {
    line.contains("{% else %}")
}

fn closes_if(line: &str) -> bool {
    line.contains("{% endif %}")
}

fn opens_for(line: &str) -> bool {
    line.contains("{% for ")
}

fn closes_for(line: &str) -> bool {
    line.contains("{% endfor %}")
}

/// Source lines (0-based) carrying a control tag that is *not* alone on its line.
///
/// Such a tag is read one way when it would open a block (ordinary text) and
/// another way when it would close or split one (a real boundary), so a template
/// free of them has an unambiguous block structure. This is the precondition
/// check for callers that want to enforce the stricter language and reject
/// ambiguous templates; [`render`] itself accepts them, so existing templates
/// keep rendering exactly as they did.
pub fn embedded_control_tags(template: &str) -> Vec<u32> {
    template
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            line.contains("{%")
                && control_inner(line)
                    .map_or(true, |inner| inner.contains("{%") || inner.contains("%}"))
        })
        .map(|(i, _)| i as u32)
        .collect()
}

/// Source lines (0-based) carrying a second or later `{% else %}` within one
/// `{% if %}` block.
///
/// The renderer treats every depth-1 `else` as a separator but anchors the
/// else-branch on the last, so with two separators it emits the text after the
/// first while attributing provenance from the last. Rendering and coverage then
/// disagree: a rule body can appear in the output while its rule is reported
/// uncovered. Any analysis that reasons from guard chains to what renders,
/// including the unreachable-rule soundness argument, loses its footing there.
/// [`render`] rejects such templates so the precondition is enforced rather
/// than assumed.
pub fn duplicate_else_lines(template: &str) -> Vec<u32> {
    let lines: Vec<&str> = template.lines().collect();
    let mut dupes = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if !opens_if(line) {
            continue;
        }
        let bounds = scan_if_block(&lines, i, lines.len());
        dupes.extend(bounds.else_lines.iter().skip(1).map(|&l| l as u32));
    }
    dupes.sort_unstable();
    dupes.dedup();
    dupes
}

/// Boundaries of the `{% if %}` block opened at `start`.
struct IfBounds {
    /// Lines carrying a depth-1 `{% else %}`, in source order. A malformed
    /// template can have more than one; the renderer treats every one of them as
    /// a separator and anchors the else-branch on the last.
    else_lines: Vec<usize>,
    /// Index of the closing `{% endif %}`, or `limit` when the block is unclosed.
    end: usize,
}

/// Find the extent of the `{% if %}` block opened at `start`, searching no
/// further than `limit`.
fn scan_if_block(lines: &[&str], start: usize, limit: usize) -> IfBounds {
    let mut i = start + 1;
    let mut depth = 1usize;
    let mut else_lines = Vec::new();

    while i < limit {
        let line = lines[i];
        if closes_if(line) {
            depth -= 1;
            if depth == 0 {
                break;
            }
        } else if opens_if(line) {
            depth += 1;
        } else if is_else(line) && depth == 1 {
            else_lines.push(i);
        }
        i += 1;
    }

    IfBounds { else_lines, end: i }
}

/// Index of the `{% endfor %}` closing the block opened at `start`, or `limit`
/// when the block is unclosed.
fn scan_for_block(lines: &[&str], start: usize, limit: usize) -> usize {
    let mut i = start + 1;
    let mut depth = 1usize;

    while i < limit {
        let line = lines[i];
        if closes_for(line) {
            depth -= 1;
            if depth == 0 {
                break;
            }
        } else if opens_for(line) {
            depth += 1;
        }
        i += 1;
    }
    i
}

/// A single enclosing guard on a source line.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Guard {
    /// Inside the true-branch of the `{% if %}` opened on this 0-based line.
    IfTrue(u32),
    /// Inside the else-branch of the `{% if %}` opened on this 0-based line.
    IfElse(u32),
    /// Inside the body of the `{% for %}` opened on this 0-based line.
    For(u32),
}

/// The enclosing guard chain (`{% if %}` / `{% else %}` / `{% for %}` context) of
/// every source line, computed with exactly the block boundaries the renderer
/// uses.
///
/// Two lines with equal chains render under identical conditions: whenever one is
/// emitted the other is too. That is the property static analysis over rules
/// depends on, and the reason this lives beside the renderer instead of being
/// re-derived by each consumer.
pub fn guard_chains(template: &str) -> Vec<Vec<Guard>> {
    let lines: Vec<&str> = template.lines().collect();
    let mut chains = vec![Vec::new(); lines.len()];
    let mut stack: Vec<Guard> = Vec::new();
    walk_guards(&lines, 0, lines.len(), &mut stack, &mut chains);
    chains
}

/// Walk `lines[from..to]` the way the renderer does — dispatch on
/// [`is_control_line`], delimit blocks with [`scan_if_block`] / [`scan_for_block`]
/// — recording the guard stack in force on each line.
fn walk_guards(
    lines: &[&str],
    from: usize,
    to: usize,
    stack: &mut Vec<Guard>,
    chains: &mut [Vec<Guard>],
) {
    let mut i = from;
    while i < to {
        chains[i] = stack.clone();

        let Some(inner) = (if is_control_line(lines[i]) {
            control_inner(lines[i])
        } else {
            None
        }) else {
            // Ordinary text, or a line the renderer would skip as an unknown
            // control block. Either way it opens nothing.
            i += 1;
            continue;
        };

        if inner.strip_prefix("if ").is_some() {
            let bounds = scan_if_block(lines, i, to);
            let if_end = bounds.else_lines.first().copied().unwrap_or(bounds.end);
            stack.push(Guard::IfTrue(i as u32));
            walk_guards(lines, i + 1, if_end.min(to), stack, chains);
            stack.pop();

            if let Some(first_else) = bounds.else_lines.first().copied() {
                chains[first_else] = stack.clone();
                stack.push(Guard::IfElse(i as u32));
                walk_guards(lines, first_else + 1, bounds.end.min(to), stack, chains);
                stack.pop();
            }
            if bounds.end < to {
                chains[bounds.end] = stack.clone();
            }
            i = bounds.end + 1;
            continue;
        }

        if inner.strip_prefix("for ").is_some() {
            let end = scan_for_block(lines, i, to);
            stack.push(Guard::For(i as u32));
            walk_guards(lines, i + 1, end.min(to), stack, chains);
            stack.pop();
            if end < to {
                chains[end] = stack.clone();
            }
            i = end + 1;
            continue;
        }

        // A control tag the renderer does not act on (`{% elif %}`, a stray
        // `{% endif %}`): it is skipped, and it changes no guard.
        i += 1;
    }
}

/// Render a template with the given context.
pub fn render(template: &str, context: &Value) -> Result<RenderResult, TemplateError> {
    // Enforce the block-structure precondition the guard-chain analyses rely on.
    if let Some(&line) = duplicate_else_lines(template).first() {
        return Err(TemplateError::ParseError {
            line: line + 1,
            message: "a second `{% else %}` in one `{% if %}` block makes rendering \
                      and source provenance disagree; split it into separate blocks"
                .to_string(),
        });
    }

    let mut ctx = RenderContext::new(context);
    let output = render_internal(template, &mut ctx)?;

    let mut rendered_lines = ctx.rendered_lines;
    rendered_lines.sort_unstable();
    rendered_lines.dedup();

    Ok(RenderResult {
        output,
        variables_used: ctx.variables_used,
        branches_taken: ctx.branches_taken,
        warnings: ctx.warnings,
        variable_accesses: ctx.variable_accesses,
        loop_iterations: ctx.loop_iterations,
        rendered_lines,
    })
}

fn render_internal(template: &str, ctx: &mut RenderContext) -> Result<String, TemplateError> {
    let mut output = String::new();
    let lines: Vec<&str> = template.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        ctx.current_line = ctx.line_offset + i as u32;
        let line = lines[i];
        let trimmed = line.trim();

        // Annotation comments are toolchain metadata, not prompt content. Record the
        // line as covered (it sits in an active, taken region) for coverage analysis,
        // but omit it from the rendered prompt the model actually sees. This also lets
        // annotation-only mutations be invisible to an output oracle yet caught by the
        // structural trace oracle.
        if trimmed.starts_with("// @") {
            ctx.rendered_lines.push(ctx.line_offset + i as u32);
            i += 1;
            continue;
        }

        // Handle control flow
        if is_control_line(line) {
            let (new_i, block_output) = process_control_block(&lines, i, ctx)?;
            output.push_str(&block_output);
            i = new_i;
            continue;
        }

        // Process variable interpolations
        let abs_line = ctx.line_offset + i as u32;
        ctx.rendered_lines.push(abs_line);
        let processed_line = interpolate_variables(line, ctx, abs_line)?;
        output.push_str(&processed_line);
        output.push('\n');

        i += 1;
    }

    // Remove trailing newline if present
    if output.ends_with('\n') {
        output.pop();
    }

    Ok(output)
}

/// Parse annotations from template content.
pub fn parse_annotations(template: &str) -> Vec<Annotation> {
    let mut annotations = Vec::new();

    for (line_num, line) in template.lines().enumerate() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("// @") {
            if let Some(annotation) = parse_annotation_line(rest, line_num as u32) {
                annotations.push(annotation);
            }
        }
    }

    annotations
}

fn parse_annotation_line(content: &str, line: u32) -> Option<Annotation> {
    let parts: Vec<&str> = content.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }

    let (kind, id) = if parts[0].starts_with("rule") || parts[0].starts_with("RULE") {
        (AnnotationKind::Rule, parts.get(1).map(|s| s.to_string()))
    } else if parts[0].starts_with("condition") || parts[0].starts_with("CONDITION") {
        (
            AnnotationKind::Condition,
            parts.get(1).map(|s| s.to_string()),
        )
    } else if parts[0].starts_with("breakpoint") || parts[0].starts_with("BREAKPOINT") {
        (
            AnnotationKind::Breakpoint,
            parts.get(1).map(|s| s.to_string()),
        )
    } else if parts[0].starts_with("provider") {
        (
            AnnotationKind::Provider,
            parts.get(1).map(|s| s.to_string()),
        )
    } else {
        return None;
    };

    let id = id.unwrap_or_else(|| format!("{}:{}", parts[0].to_uppercase(), line));

    // Parse key=value attributes
    let mut attributes = HashMap::new();
    for part in parts.iter().skip(2) {
        if let Some((key, value)) = part.split_once('=') {
            attributes.insert(key.to_string(), value.to_string());
        }
    }

    Some(Annotation {
        kind,
        id,
        attributes,
        range: TextRange::new(line, 0, line, content.len() as u32),
    })
}

/// Engine identifier recorded in [`prompt_ir::IRMeta`] for the deterministic
/// annotation-based IR builder. Distinguishes it from the `"llm"` engine used by
/// the SLM analysis path.
pub const ANNOTATION_ENGINE: &str = "annotation";

/// Build a deterministic [`PromptIR`] from a template's explicit annotations.
///
/// Unlike the LLM-backed `generate_ir`, this performs **no inference**: it maps every
/// `// @rule` / `// @condition` / `// @breakpoint` / `// @provider` annotation to an IR
/// node, in source order, so the same input always produces byte-identical IR. This is the
/// reproducible foundation for coverage and mutation measurement, and it requires no
/// provider or API key.
///
/// Annotation attributes are mapped onto [`IRNodeMeta`]:
/// `category=` → `category`, `trigger=`/`triggers=` (comma-separated) → `triggers`,
/// `conditionRef=`/`condition_ref=` → `condition_ref`, `priority=` (integer) → `priority`.
/// All nodes are marked `inferred = false`.
pub fn build_ir(template: &str, uri: impl Into<String>, version: i32) -> PromptIR {
    let annotations = parse_annotations(template);
    let mut nodes: Vec<IRNode> = Vec::with_capacity(annotations.len());
    let mut warnings: Vec<String> = Vec::new();
    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for ann in &annotations {
        let kind = match ann.kind {
            AnnotationKind::Rule => IRKind::Rule,
            AnnotationKind::Condition => IRKind::Condition,
            AnnotationKind::Breakpoint => IRKind::Breakpoint,
            AnnotationKind::Provider => IRKind::Directive,
        };

        if !seen_ids.insert(ann.id.clone()) {
            warnings.push(format!(
                "duplicate node id '{}' at line {}",
                ann.id,
                ann.range.start_line + 1
            ));
        }

        let triggers = ann
            .attributes
            .get("trigger")
            .or_else(|| ann.attributes.get("triggers"))
            .map(|s| {
                s.split(',')
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let condition_ref = ann
            .attributes
            .get("conditionRef")
            .or_else(|| ann.attributes.get("condition_ref"))
            .cloned();

        let priority = ann
            .attributes
            .get("priority")
            .and_then(|p| p.parse::<i32>().ok());

        nodes.push(IRNode {
            id: ann.id.clone(),
            kind,
            label: None,
            range: ann.range,
            meta: IRNodeMeta {
                category: ann.attributes.get("category").cloned(),
                triggers,
                condition_ref,
                priority,
                inferred: false,
            },
        });
    }

    // Flag breakpoint condition references that don't resolve to a condition node.
    let condition_ids: std::collections::HashSet<&str> = nodes
        .iter()
        .filter(|n| n.kind == IRKind::Condition)
        .map(|n| n.id.as_str())
        .collect();
    for node in &nodes {
        if node.kind == IRKind::Breakpoint {
            if let Some(cref) = &node.meta.condition_ref {
                if !condition_ids.contains(cref.as_str()) {
                    warnings.push(format!(
                        "breakpoint '{}' references unknown condition '{}'",
                        node.id, cref
                    ));
                }
            }
        }
    }

    PromptIR {
        uri: uri.into(),
        version,
        nodes,
        meta: Some(IRMeta {
            parser_version: env!("CARGO_PKG_VERSION").to_string(),
            confidence: 1.0,
            engine: ANNOTATION_ENGINE.to_string(),
            warnings,
        }),
    }
}

fn interpolate_variables(
    line: &str,
    ctx: &mut RenderContext,
    line_num: u32,
) -> Result<String, TemplateError> {
    let mut result = String::new();
    let mut chars = line.chars().peekable();
    let mut column: u32 = 0;

    while let Some(c) = chars.next() {
        if c == '{' && chars.peek() == Some(&'{') {
            chars.next(); // consume second {
            let var_start_col = column;
            column += 2;

            // Find closing }}
            let mut var_name = String::new();
            let mut found_close = false;

            while let Some(c) = chars.next() {
                column += 1;
                if c == '}' && chars.peek() == Some(&'}') {
                    chars.next(); // consume second }
                    column += 1;
                    found_close = true;
                    break;
                }
                var_name.push(c);
            }

            if !found_close {
                return Err(TemplateError::UnclosedBlock("{{".to_string()));
            }

            let var_name = var_name.trim();
            ctx.variables_used.push(var_name.to_string());

            // Look up variable in context
            let value = get_value_by_path(ctx.context, var_name)
                .ok_or_else(|| TemplateError::MissingVariable(var_name.to_string()))?;

            // Record access for debugging
            ctx.record_variable_access(var_name, value, line_num, var_start_col);

            match value {
                Value::String(s) => result.push_str(s),
                Value::Number(n) => result.push_str(&n.to_string()),
                Value::Bool(b) => result.push_str(&b.to_string()),
                Value::Null => result.push_str("null"),
                _ => result.push_str(&value.to_string()),
            }
        } else {
            result.push(c);
            column += 1;
        }
    }

    Ok(result)
}

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

fn process_control_block(
    lines: &[&str],
    start: usize,
    ctx: &mut RenderContext,
) -> Result<(usize, String), TemplateError> {
    let line = lines[start].trim();

    // Parse {% if condition %}
    if let Some(condition) = line
        .strip_prefix("{%")
        .and_then(|s| s.strip_suffix("%}"))
        .map(|s| s.trim())
        .and_then(|s| s.strip_prefix("if "))
    {
        return process_if_block(lines, start, condition.trim(), ctx);
    }

    // Parse {% for item in collection %}
    if let Some(for_expr) = line
        .strip_prefix("{%")
        .and_then(|s| s.strip_suffix("%}"))
        .map(|s| s.trim())
        .and_then(|s| s.strip_prefix("for "))
    {
        return process_for_block(lines, start, for_expr.trim(), ctx);
    }

    // Unknown control block - skip line
    Ok((start + 1, String::new()))
}

fn process_if_block(
    lines: &[&str],
    start: usize,
    condition: &str,
    ctx: &mut RenderContext,
) -> Result<(usize, String), TemplateError> {
    // Block boundaries come from the shared scanner, so the renderer and
    // `guard_chains` agree by construction about where this block ends and where
    // its else-branch starts.
    let bounds = scan_if_block(lines, start, lines.len());
    let first_else = bounds.else_lines.first().copied();
    let else_line = bounds.else_lines.last().copied();
    let i = bounds.end;

    let mut if_content = String::new();
    let mut else_content = String::new();
    for (idx, line) in lines
        .iter()
        .enumerate()
        .take(i.min(lines.len()))
        .skip(start + 1)
    {
        if bounds.else_lines.contains(&idx) {
            continue;
        }
        let target = match first_else {
            Some(e) if idx > e => &mut else_content,
            _ => &mut if_content,
        };
        target.push_str(line);
        target.push('\n');
    }

    // Evaluate condition
    let (condition_result, evaluated_value) = evaluate_condition(condition, ctx.context);

    let branch_executed = if condition_result {
        "if"
    } else if !else_content.is_empty() {
        "else"
    } else {
        "none"
    };

    let base = ctx.line_offset;
    ctx.branches_taken.push(BranchInfo {
        condition: condition.to_string(),
        taken: condition_result,
        range: TextRange::new(base + start as u32, 0, base + i as u32, 0),
        condition_line: base + start as u32,
        evaluated_value: Some(evaluated_value),
        branch_executed: branch_executed.to_string(),
    });

    // Absolute source line of the first line of the branch body we are about to
    // render: the line after `{% if %}` for the if-side, or after `{% else %}`.
    let (content, body_offset) = if condition_result {
        (&if_content, base + start as u32 + 1)
    } else {
        let else_offset = else_line
            .map(|l| base + l as u32 + 1)
            .unwrap_or(base + start as u32 + 1);
        (&else_content, else_offset)
    };

    // Recursively render the chosen branch with the offset pointing at its
    // absolute start, then restore so sibling content keeps the parent offset.
    let mut output = String::new();
    if !content.is_empty() {
        ctx.line_offset = body_offset;
        output = render_internal(content, ctx)?;
        ctx.line_offset = base;
        output.push('\n');
    }

    Ok((i + 1, output))
}

fn process_for_block(
    lines: &[&str],
    start: usize,
    for_expr: &str,
    ctx: &mut RenderContext,
) -> Result<(usize, String), TemplateError> {
    // Parse "item in collection"
    let parts: Vec<&str> = for_expr.split(" in ").collect();
    if parts.len() != 2 {
        return Err(TemplateError::InvalidExpression(format!(
            "Invalid for expression: {}",
            for_expr
        )));
    }

    let var_name = parts[0].trim();
    let collection_name = parts[1].trim();

    // Find endfor with the shared scanner (see `scan_for_block`).
    let i = scan_for_block(lines, start, lines.len());
    let mut loop_content = String::new();
    for line in &lines[(start + 1).min(lines.len())..i.min(lines.len())] {
        loop_content.push_str(line);
        loop_content.push('\n');
    }

    // Get collection value
    let collection = get_value_by_path(ctx.context, collection_name);
    let items = match collection {
        Some(Value::Array(arr)) => arr.clone(),
        Some(_) => {
            return Err(TemplateError::InvalidExpression(format!(
                "{} is not an array",
                collection_name
            )));
        }
        None => {
            return Err(TemplateError::MissingVariable(collection_name.to_string()));
        }
    };

    // Record loop info
    let base = ctx.line_offset;
    ctx.loop_iterations.push(LoopInfo {
        variable: var_name.to_string(),
        collection: collection_name.to_string(),
        iteration_count: items.len(),
        range: TextRange::new(base + start as u32, 0, base + i as u32, 0),
    });

    // The loop body begins on the line after `{% for %}`; every iteration maps
    // back to the same absolute source lines.
    let body_offset = base + start as u32 + 1;

    // Iterate and render
    let mut output = String::new();
    for item in &items {
        // Create a new context with the loop variable
        let loop_context = match ctx.context {
            Value::Object(map) => {
                let mut new_map = map.clone();
                new_map.insert(var_name.to_string(), item.clone());
                Value::Object(new_map)
            }
            _ => ctx.context.clone(),
        };

        let mut inner_ctx = RenderContext::new(&loop_context);
        inner_ctx.line_offset = body_offset;
        let rendered = render_internal(&loop_content, &mut inner_ctx)?;
        output.push_str(&rendered);
        output.push('\n');

        // Merge tracking info
        ctx.variables_used.extend(inner_ctx.variables_used);
        ctx.variable_accesses.extend(inner_ctx.variable_accesses);
        ctx.branches_taken.extend(inner_ctx.branches_taken);
        ctx.loop_iterations.extend(inner_ctx.loop_iterations);
        ctx.rendered_lines.extend(inner_ctx.rendered_lines);
    }

    Ok((i + 1, output))
}

fn evaluate_condition(condition: &str, context: &Value) -> (bool, String) {
    // Handle comparison operators
    if let Some((left, right)) = condition.split_once("==") {
        let left_val = get_condition_value(left.trim(), context);
        let right_val = get_condition_value(right.trim(), context);
        let result = left_val == right_val;
        return (
            result,
            format!("{} == {} → {}", left_val, right_val, result),
        );
    }

    if let Some((left, right)) = condition.split_once("!=") {
        let left_val = get_condition_value(left.trim(), context);
        let right_val = get_condition_value(right.trim(), context);
        let result = left_val != right_val;
        return (
            result,
            format!("{} != {} → {}", left_val, right_val, result),
        );
    }

    // Handle negation
    if let Some(inner) = condition
        .strip_prefix("not ")
        .or_else(|| condition.strip_prefix("!"))
    {
        let (inner_result, _) = evaluate_condition(inner.trim(), context);
        let result = !inner_result;
        return (result, format!("not {} → {}", inner.trim(), result));
    }

    // Simple truthy check
    if let Some(value) = get_value_by_path(context, condition) {
        let result = match value {
            Value::Bool(b) => *b,
            Value::Null => false,
            Value::String(s) => !s.is_empty(),
            Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
            Value::Array(a) => !a.is_empty(),
            Value::Object(o) => !o.is_empty(),
        };
        (
            result,
            format!("{} = {} → {}", condition, format_value(value), result),
        )
    } else {
        (false, format!("{} = undefined → false", condition))
    }
}

fn get_condition_value(expr: &str, context: &Value) -> String {
    // Check if it's a string literal
    if (expr.starts_with('"') && expr.ends_with('"'))
        || (expr.starts_with('\'') && expr.ends_with('\''))
    {
        return expr[1..expr.len() - 1].to_string();
    }

    // Check if it's a number
    if let Ok(n) = expr.parse::<f64>() {
        return n.to_string();
    }

    // Check if it's a boolean literal
    if expr == "true" {
        return "true".to_string();
    }
    if expr == "false" {
        return "false".to_string();
    }

    // Otherwise it's a variable path
    if let Some(value) = get_value_by_path(context, expr) {
        format_value(value)
    } else {
        "undefined".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_simple_interpolation() {
        let template = "Hello {{ name }}!";
        let context = json!({ "name": "World" });
        let result = render(template, &context).unwrap();
        assert_eq!(result.output, "Hello World!");
        assert_eq!(result.variables_used, vec!["name"]);
        assert_eq!(result.variable_accesses.len(), 1);
        assert_eq!(result.variable_accesses[0].value, "World");
    }

    #[test]
    fn test_nested_variable() {
        let template = "User: {{ user.name }}";
        let context = json!({ "user": { "name": "Alice" } });
        let result = render(template, &context).unwrap();
        assert_eq!(result.output, "User: Alice");
    }

    #[test]
    fn test_if_block() {
        let template = "{% if show_greeting %}\nHello!\n{% endif %}";
        let context = json!({ "show_greeting": true });
        let result = render(template, &context).unwrap();
        assert!(result.output.contains("Hello!"));
        assert_eq!(result.branches_taken.len(), 1);
        assert!(result.branches_taken[0].taken);
        assert_eq!(result.branches_taken[0].branch_executed, "if");

        let context_false = json!({ "show_greeting": false });
        let result_false = render(template, &context_false).unwrap();
        assert!(!result_false.output.contains("Hello!"));
        assert!(!result_false.branches_taken[0].taken);
    }

    #[test]
    fn test_if_else_block() {
        let template = "{% if formal %}\nDear Sir,\n{% else %}\nHey!\n{% endif %}";

        let formal = json!({ "formal": true });
        let result = render(template, &formal).unwrap();
        assert!(result.output.contains("Dear Sir"));
        assert_eq!(result.branches_taken[0].branch_executed, "if");

        let casual = json!({ "formal": false });
        let result = render(template, &casual).unwrap();
        assert!(result.output.contains("Hey!"));
        assert_eq!(result.branches_taken[0].branch_executed, "else");
    }

    #[test]
    fn test_for_loop() {
        let template = "{% for item in items %}\n- {{ item }}\n{% endfor %}";
        let context = json!({ "items": ["apple", "banana", "cherry"] });
        let result = render(template, &context).unwrap();
        assert!(result.output.contains("apple"));
        assert!(result.output.contains("banana"));
        assert!(result.output.contains("cherry"));
        assert_eq!(result.loop_iterations.len(), 1);
        assert_eq!(result.loop_iterations[0].iteration_count, 3);
    }

    #[test]
    fn test_comparison_condition() {
        let template = "{% if status == \"active\" %}\nActive!\n{% endif %}";
        let context = json!({ "status": "active" });
        let result = render(template, &context).unwrap();
        assert!(result.output.contains("Active!"));
        assert!(result.branches_taken[0]
            .evaluated_value
            .as_ref()
            .unwrap()
            .contains("=="));
    }

    /// `guard_chains` must agree with the renderer line for line: a line's chain
    /// says exactly which branch decisions have to go its way for it to be emitted.
    #[test]
    fn test_guard_chains_agree_with_renderer() {
        // The line under test, the guard chain claimed for it, and a context that
        // satisfies every guard in that chain.
        let cases: Vec<(&str, usize, Vec<Guard>, Value)> = vec![
            (
                "{% if a %}\nyes\n{% else %}\nno\n{% endif %}",
                1,
                vec![Guard::IfTrue(0)],
                json!({ "a": true }),
            ),
            (
                "{% if a %}\nyes\n{% else %}\nno\n{% endif %}",
                3,
                vec![Guard::IfElse(0)],
                json!({ "a": false }),
            ),
            (
                "{% if a %}\n{% if b %}\ndeep\n{% endif %}\n{% endif %}",
                2,
                vec![Guard::IfTrue(0), Guard::IfTrue(1)],
                json!({ "a": true, "b": true }),
            ),
            (
                "{% for x in xs %}\nitem\n{% endfor %}",
                1,
                vec![Guard::For(0)],
                json!({ "xs": [1] }),
            ),
            (
                "top\n{% if a %}\nguarded\n{% endif %}",
                0,
                vec![],
                json!({ "a": false }),
            ),
        ];

        for (template, line, expected, ctx) in cases {
            let chains = guard_chains(template);
            assert_eq!(chains[line], expected, "chain for {template:?} line {line}");
            let text = template.lines().nth(line).unwrap();
            assert!(
                render(template, &ctx).unwrap().output.contains(text),
                "{text:?} should render under {ctx}"
            );
        }
    }

    /// A `{% for %}` nested inside an `{% if %}` must not close the `{% if %}`:
    /// the renderer's if-scanner ignores `{% endfor %}` entirely, and the guard
    /// chains have to model that, not a single mixed tag stack.
    #[test]
    fn test_guard_chains_track_block_types_separately() {
        let t = "{% if a %}\n{% for x in xs %}\nloop\n{% endfor %}\nafter\n{% else %}\nelse\n{% endif %}";
        let chains = guard_chains(t);
        assert_eq!(chains[2], vec![Guard::IfTrue(0), Guard::For(1)]);
        // `after` is still inside the if — the `{% endfor %}` closed only the loop.
        assert_eq!(chains[4], vec![Guard::IfTrue(0)]);
        assert_eq!(chains[6], vec![Guard::IfElse(0)]);

        let out = render(t, &json!({ "a": true, "xs": [1] })).unwrap().output;
        assert!(out.contains("after") && !out.contains("else"), "{out:?}");
    }

    /// An `{% else %}` embedded in prose separates the branches for the renderer,
    /// so `guard_chains` reports it as a separator too.
    #[test]
    fn test_guard_chains_see_embedded_else() {
        let t = "{% if flag %}\nD body\nliteral {% else %} marker\nR body\n{% endif %}";
        let chains = guard_chains(t);
        assert_eq!(chains[1], vec![Guard::IfTrue(0)]);
        assert_eq!(chains[3], vec![Guard::IfElse(0)]);

        let out = render(t, &json!({ "flag": false })).unwrap().output;
        assert!(out.contains("R body") && !out.contains("D body"), "{out:?}");
    }

    #[test]
    fn test_embedded_control_tags() {
        assert!(embedded_control_tags("{% if a %}\nx\n{% endif %}").is_empty());
        assert!(embedded_control_tags("  {% endfor %}  \nplain text").is_empty());
        assert_eq!(embedded_control_tags("a\n* {% for x in xs %}"), vec![1]);
        assert_eq!(embedded_control_tags("lit {% else %} marker"), vec![0]);
        assert_eq!(embedded_control_tags("{% if a %}{% endif %}"), vec![0]);
        assert_eq!(embedded_control_tags("{% if a %}trailing"), vec![0]);

        assert!(is_control_line("  {% if a %}"));
        assert!(!is_control_line("* {% if a %}"));
    }

    #[test]
    fn test_parse_annotations() {
        let template = r#"// @rule RULE:GREETING category=style
You are friendly.
// @condition CONDITION:FORMAL
// @breakpoint BREAKPOINT:START"#;

        let annotations = parse_annotations(template);
        assert_eq!(annotations.len(), 3);
        assert_eq!(annotations[0].kind, AnnotationKind::Rule);
        assert_eq!(annotations[0].id, "RULE:GREETING");
        assert_eq!(
            annotations[0].attributes.get("category"),
            Some(&"style".to_string())
        );
    }

    #[test]
    fn test_missing_variable() {
        let template = "Hello {{ name }}!";
        let context = json!({});
        let result = render(template, &context);
        assert!(matches!(result, Err(TemplateError::MissingVariable(_))));
    }

    // ==========================================
    // Branch Coverage Tests
    // ==========================================

    #[test]
    fn test_nested_if_blocks() {
        let template = r#"{% if outer %}
{% if inner %}
Both true
{% endif %}
{% endif %}"#;

        // Both true
        let ctx = json!({ "outer": true, "inner": true });
        let result = render(template, &ctx).unwrap();
        assert!(result.output.contains("Both true"));
        assert_eq!(result.branches_taken.len(), 2);
        assert!(result.branches_taken.iter().all(|b| b.taken));

        // Outer true, inner false
        let ctx = json!({ "outer": true, "inner": false });
        let result = render(template, &ctx).unwrap();
        assert!(!result.output.contains("Both true"));
        assert!(result.branches_taken[0].taken);
        assert!(!result.branches_taken[1].taken);

        // Outer false - inner not evaluated
        let ctx = json!({ "outer": false, "inner": true });
        let result = render(template, &ctx).unwrap();
        assert!(!result.output.contains("Both true"));
        assert_eq!(result.branches_taken.len(), 1);
        assert!(!result.branches_taken[0].taken);
    }

    #[test]
    fn test_sequential_if_blocks() {
        let template = r#"{% if a %}
A is true
{% endif %}
{% if b %}
B is true
{% endif %}
{% if c %}
C is true
{% endif %}"#;

        let ctx = json!({ "a": true, "b": false, "c": true });
        let result = render(template, &ctx).unwrap();
        assert!(result.output.contains("A is true"));
        assert!(!result.output.contains("B is true"));
        assert!(result.output.contains("C is true"));
        assert_eq!(result.branches_taken.len(), 3);
        assert!(result.branches_taken[0].taken);
        assert!(!result.branches_taken[1].taken);
        assert!(result.branches_taken[2].taken);
    }

    #[test]
    fn test_negation_condition() {
        let template = r#"{% if not disabled %}
Feature enabled
{% endif %}"#;

        let ctx = json!({ "disabled": false });
        let result = render(template, &ctx).unwrap();
        assert!(result.output.contains("Feature enabled"));
        assert!(result.branches_taken[0].taken);
        assert!(result.branches_taken[0]
            .evaluated_value
            .as_ref()
            .unwrap()
            .contains("not"));

        let ctx = json!({ "disabled": true });
        let result = render(template, &ctx).unwrap();
        assert!(!result.output.contains("Feature enabled"));
        assert!(!result.branches_taken[0].taken);
    }

    #[test]
    fn test_bang_negation() {
        let template = r#"{% if !hidden %}
Visible content
{% endif %}"#;

        let ctx = json!({ "hidden": false });
        let result = render(template, &ctx).unwrap();
        assert!(result.output.contains("Visible content"));

        let ctx = json!({ "hidden": true });
        let result = render(template, &ctx).unwrap();
        assert!(!result.output.contains("Visible content"));
    }

    #[test]
    fn test_not_equal_condition() {
        let template = r#"{% if status != "inactive" %}
Service running
{% endif %}"#;

        let ctx = json!({ "status": "active" });
        let result = render(template, &ctx).unwrap();
        assert!(result.output.contains("Service running"));
        assert!(result.branches_taken[0]
            .evaluated_value
            .as_ref()
            .unwrap()
            .contains("!="));

        let ctx = json!({ "status": "inactive" });
        let result = render(template, &ctx).unwrap();
        assert!(!result.output.contains("Service running"));
    }

    #[test]
    fn test_undefined_condition_variable() {
        let template = r#"{% if undefined_var %}
Should not appear
{% else %}
Fallback content
{% endif %}"#;

        let ctx = json!({});
        let result = render(template, &ctx).unwrap();
        assert!(!result.output.contains("Should not appear"));
        assert!(result.output.contains("Fallback content"));
        assert!(!result.branches_taken[0].taken);
        assert_eq!(result.branches_taken[0].branch_executed, "else");
        assert!(result.branches_taken[0]
            .evaluated_value
            .as_ref()
            .unwrap()
            .contains("undefined"));
    }

    #[test]
    fn test_empty_string_condition() {
        let template = r#"{% if message %}
Has message: {{ message }}
{% else %}
No message
{% endif %}"#;

        let ctx = json!({ "message": "Hello" });
        let result = render(template, &ctx).unwrap();
        assert!(result.output.contains("Has message: Hello"));
        assert!(result.branches_taken[0].taken);

        let ctx = json!({ "message": "" });
        let result = render(template, &ctx).unwrap();
        assert!(result.output.contains("No message"));
        assert!(!result.branches_taken[0].taken);
    }

    #[test]
    fn test_numeric_condition() {
        let template = r#"{% if count %}
Count is non-zero
{% endif %}"#;

        let ctx = json!({ "count": 5 });
        let result = render(template, &ctx).unwrap();
        assert!(result.output.contains("Count is non-zero"));

        let ctx = json!({ "count": 0 });
        let result = render(template, &ctx).unwrap();
        assert!(!result.output.contains("Count is non-zero"));
    }

    #[test]
    fn test_array_condition() {
        let template = r#"{% if items %}
Has items
{% else %}
Empty list
{% endif %}"#;

        let ctx = json!({ "items": ["a", "b"] });
        let result = render(template, &ctx).unwrap();
        assert!(result.output.contains("Has items"));

        let ctx = json!({ "items": [] });
        let result = render(template, &ctx).unwrap();
        assert!(result.output.contains("Empty list"));
    }

    #[test]
    fn test_null_condition() {
        let template = r#"{% if value %}
Has value
{% else %}
Null value
{% endif %}"#;

        let ctx = json!({ "value": null });
        let result = render(template, &ctx).unwrap();
        assert!(result.output.contains("Null value"));
        assert!(!result.branches_taken[0].taken);
    }

    // ==========================================
    // Loop Coverage Tests
    // ==========================================

    #[test]
    fn test_empty_loop() {
        let template = r#"{% for item in items %}
- {{ item }}
{% endfor %}
Done"#;

        let ctx = json!({ "items": [] });
        let result = render(template, &ctx).unwrap();
        assert!(result.output.contains("Done"));
        assert!(!result.output.contains("-"));
        assert_eq!(result.loop_iterations.len(), 1);
        assert_eq!(result.loop_iterations[0].iteration_count, 0);
    }

    #[test]
    fn test_nested_loops() {
        let template = r#"{% for group in groups %}
Group: {{ group.name }}
{% for member in group.members %}
  - {{ member }}
{% endfor %}
{% endfor %}"#;

        let ctx = json!({
            "groups": [
                { "name": "A", "members": ["Alice", "Amy"] },
                { "name": "B", "members": ["Bob"] }
            ]
        });
        let result = render(template, &ctx).unwrap();
        assert!(result.output.contains("Group: A"));
        assert!(result.output.contains("Group: B"));
        assert!(result.output.contains("Alice"));
        assert!(result.output.contains("Amy"));
        assert!(result.output.contains("Bob"));
        // Outer loop + 2 inner loops (one per group)
        assert_eq!(result.loop_iterations.len(), 3);
    }

    #[test]
    fn test_loop_with_conditional() {
        let template = r#"{% for task in tasks %}
{% if task.urgent %}
[URGENT] {{ task.name }}
{% else %}
{{ task.name }}
{% endif %}
{% endfor %}"#;

        let ctx = json!({
            "tasks": [
                { "name": "Fix bug", "urgent": true },
                { "name": "Update docs", "urgent": false },
                { "name": "Deploy", "urgent": true }
            ]
        });
        let result = render(template, &ctx).unwrap();
        assert!(result.output.contains("[URGENT] Fix bug"));
        assert!(result.output.contains("Update docs"));
        assert!(!result.output.contains("[URGENT] Update docs"));
        assert!(result.output.contains("[URGENT] Deploy"));
        // 3 branches (one per loop iteration)
        assert_eq!(result.branches_taken.len(), 3);
        assert!(result.branches_taken[0].taken); // urgent
        assert!(!result.branches_taken[1].taken); // not urgent
        assert!(result.branches_taken[2].taken); // urgent
    }

    #[test]
    fn test_loop_with_object_items() {
        let template = r#"{% for user in users %}
{{ user.name }} ({{ user.role }})
{% endfor %}"#;

        let ctx = json!({
            "users": [
                { "name": "Alice", "role": "admin" },
                { "name": "Bob", "role": "user" }
            ]
        });
        let result = render(template, &ctx).unwrap();
        assert!(result.output.contains("Alice (admin)"));
        assert!(result.output.contains("Bob (user)"));
    }

    // ==========================================
    // Variable Access Coverage Tests
    // ==========================================

    #[test]
    fn test_multiple_accesses_same_variable() {
        let template = r#"First: {{ name }}
Second: {{ name }}
Third: {{ name }}"#;

        let ctx = json!({ "name": "Test" });
        let result = render(template, &ctx).unwrap();
        assert_eq!(result.variable_accesses.len(), 3);
        assert!(result.variable_accesses.iter().all(|v| v.name == "name"));
        assert!(result.variable_accesses.iter().all(|v| v.value == "Test"));
    }

    #[test]
    fn test_variable_type_tracking() {
        let template = r#"String: {{ s }}
Number: {{ n }}
Bool: {{ b }}
Null: {{ null_val }}"#;

        let ctx = json!({
            "s": "hello",
            "n": 42,
            "b": true,
            "null_val": null
        });
        let result = render(template, &ctx).unwrap();
        assert_eq!(result.variable_accesses.len(), 4);

        let find_access = |name: &str| {
            result
                .variable_accesses
                .iter()
                .find(|v| v.name == name)
                .unwrap()
        };
        assert_eq!(find_access("s").value_type, "string");
        assert_eq!(find_access("n").value_type, "number");
        assert_eq!(find_access("b").value_type, "boolean");
        assert_eq!(find_access("null_val").value_type, "null");
    }

    #[test]
    fn test_deeply_nested_variable() {
        let template = "Value: {{ a.b.c.d }}";
        let ctx = json!({
            "a": { "b": { "c": { "d": "deep" } } }
        });
        let result = render(template, &ctx).unwrap();
        assert!(result.output.contains("Value: deep"));
        assert_eq!(result.variable_accesses[0].name, "a.b.c.d");
    }

    // ==========================================
    // Combined Scenario Coverage Tests
    // ==========================================

    #[test]
    fn test_branch_inside_branch() {
        let template = r#"{% if level1 %}
Level 1 entered
{% if level2 %}
Level 2 entered
{% else %}
Level 2 else
{% endif %}
{% else %}
Level 1 else
{% endif %}"#;

        // Test all branch combinations
        let ctx = json!({ "level1": true, "level2": true });
        let result = render(template, &ctx).unwrap();
        assert!(result.output.contains("Level 1 entered"));
        assert!(result.output.contains("Level 2 entered"));
        assert_eq!(result.branches_taken[0].branch_executed, "if");
        assert_eq!(result.branches_taken[1].branch_executed, "if");

        let ctx = json!({ "level1": true, "level2": false });
        let result = render(template, &ctx).unwrap();
        assert!(result.output.contains("Level 2 else"));
        assert_eq!(result.branches_taken[1].branch_executed, "else");

        let ctx = json!({ "level1": false, "level2": true });
        let result = render(template, &ctx).unwrap();
        assert!(result.output.contains("Level 1 else"));
        assert!(!result.output.contains("Level 2"));
        assert_eq!(result.branches_taken.len(), 1); // Only outer branch evaluated
    }

    #[test]
    fn test_comparison_with_numbers() {
        let template = r#"{% if value == 42 %}
Found the answer
{% endif %}"#;

        let ctx = json!({ "value": 42 });
        let result = render(template, &ctx).unwrap();
        assert!(result.output.contains("Found the answer"));

        let ctx = json!({ "value": 0 });
        let result = render(template, &ctx).unwrap();
        assert!(!result.output.contains("Found the answer"));
    }

    #[test]
    fn test_comparison_with_boolean_literals() {
        let template = r#"{% if enabled == true %}
Enabled
{% endif %}
{% if disabled == false %}
Also shown
{% endif %}"#;

        let ctx = json!({ "enabled": true, "disabled": false });
        let result = render(template, &ctx).unwrap();
        assert!(result.output.contains("Enabled"));
        assert!(result.output.contains("Also shown"));
    }

    #[test]
    fn test_complex_template() {
        let template = r#"// @rule RULE:GREETING
Hello {{ user.name }},

{% if user.premium %}
Thank you for being a premium member!
{% endif %}

Your tasks:
{% for task in tasks %}
{% if task.priority == "high" %}
[!] {{ task.title }}
{% else %}
- {{ task.title }}
{% endif %}
{% endfor %}

{% if not tasks %}
No tasks for today!
{% endif %}

Best regards"#;

        let ctx = json!({
            "user": { "name": "Alice", "premium": true },
            "tasks": [
                { "title": "Review PR", "priority": "high" },
                { "title": "Update docs", "priority": "low" },
                { "title": "Fix bug", "priority": "high" }
            ]
        });

        let result = render(template, &ctx).unwrap();
        assert!(result.output.contains("Hello Alice"));
        assert!(result
            .output
            .contains("Thank you for being a premium member"));
        assert!(result.output.contains("[!] Review PR"));
        assert!(result.output.contains("- Update docs"));
        assert!(result.output.contains("[!] Fix bug"));
        assert!(!result.output.contains("No tasks for today"));

        // Verify tracking
        assert!(result.variables_used.contains(&"user.name".to_string()));
        assert!(result.loop_iterations.len() == 1);
        assert!(result.loop_iterations[0].iteration_count == 3);
        // 1 premium branch + 3 task branches + 1 empty tasks check
        assert!(result.branches_taken.len() >= 4);
    }

    #[test]
    fn test_unclosed_variable_error() {
        let template = "Hello {{ name";
        let ctx = json!({ "name": "World" });
        let result = render(template, &ctx);
        assert!(matches!(result, Err(TemplateError::UnclosedBlock(_))));
    }

    #[test]
    fn test_invalid_for_expression_error() {
        let template = "{% for item %}content{% endfor %}";
        let ctx = json!({});
        let result = render(template, &ctx);
        assert!(matches!(result, Err(TemplateError::InvalidExpression(_))));
    }

    #[test]
    fn test_non_array_loop_error() {
        let template = r#"{% for item in items %}
{{ item }}
{% endfor %}"#;
        let ctx = json!({ "items": "not an array" });
        let result = render(template, &ctx);
        assert!(matches!(result, Err(TemplateError::InvalidExpression(_))));
    }

    #[test]
    fn test_branch_info_line_tracking() {
        let template = r#"Line 0
{% if condition %}
Content
{% endif %}
Line 4"#;

        let ctx = json!({ "condition": true });
        let result = render(template, &ctx).unwrap();
        // The if block starts at line 1
        assert_eq!(result.branches_taken[0].condition_line, 1);
        assert_eq!(result.branches_taken[0].range.start_line, 1);
    }

    #[test]
    fn test_nested_branch_absolute_line_numbers() {
        // Lines (0-based):
        // 0 {% if level1 %}
        // 1 Level 1 entered
        // 2 {% if level2 %}
        // 3 Level 2 entered
        // 4 {% else %}
        // 5 Level 2 else
        // 6 {% endif %}
        // 7 {% else %}
        // 8 Level 1 else
        // 9 {% endif %}
        let template = "{% if level1 %}\nLevel 1 entered\n{% if level2 %}\nLevel 2 entered\n{% else %}\nLevel 2 else\n{% endif %}\n{% else %}\nLevel 1 else\n{% endif %}";

        let ctx = json!({ "level1": true, "level2": false });
        let result = render(template, &ctx).unwrap();
        // Outer if is at absolute line 0.
        assert_eq!(result.branches_taken[0].condition_line, 0);
        // Inner if is at absolute line 2 (previously reported relative line 1).
        assert_eq!(result.branches_taken[1].condition_line, 2);
        assert_eq!(result.branches_taken[1].branch_executed, "else");
        // The inner else block (range end) spans to the inner {% endif %} at line 6.
        assert_eq!(result.branches_taken[1].range.start_line, 2);
    }

    #[test]
    fn test_nested_variable_access_absolute_line() {
        // 0 {% if show %}
        // 1 Hello {{ name }}
        // 2 {% endif %}
        let template = "{% if show %}\nHello {{ name }}\n{% endif %}";
        let ctx = json!({ "show": true, "name": "Ada" });
        let result = render(template, &ctx).unwrap();
        let access = result
            .variable_accesses
            .iter()
            .find(|a| a.name == "name")
            .expect("name access recorded");
        assert_eq!(access.line, 1);
    }

    #[test]
    fn test_rendered_lines_excludes_untaken_branch() {
        // 0 {% if admin %}
        // 1 // @rule RULE:ADMIN
        // 2 admin text
        // 3 {% else %}
        // 4 // @rule RULE:USER
        // 5 user text
        // 6 {% endif %}
        let template = "{% if admin %}\n// @rule RULE:ADMIN\nadmin text\n{% else %}\n// @rule RULE:USER\nuser text\n{% endif %}";

        let admin = render(template, &json!({ "admin": true })).unwrap();
        assert!(admin.rendered_lines.contains(&1)); // RULE:ADMIN annotation rendered
        assert!(admin.rendered_lines.contains(&2));
        assert!(!admin.rendered_lines.contains(&4)); // RULE:USER in untaken else
        assert!(!admin.rendered_lines.contains(&5));

        let user = render(template, &json!({ "admin": false })).unwrap();
        assert!(user.rendered_lines.contains(&4)); // RULE:USER annotation rendered
        assert!(!user.rendered_lines.contains(&1)); // RULE:ADMIN in untaken if
    }

    const ANNOTATED: &str = r#"// @rule RULE:GREETING category=style
// @provider openai model=gpt-4o-mini
You are a friendly assistant.
// @condition CONDITION:ADMIN
// @rule RULE:ADMIN_TONE category=tone trigger=topic:code,topic:ops priority=10
// @breakpoint BREAKPOINT:BEFORE_ANSWER conditionRef=CONDITION:ADMIN
Now answer."#;

    #[test]
    fn test_build_ir_kinds_and_count() {
        let ir = build_ir(ANNOTATED, "file:///t.rtpl", 1);
        assert_eq!(ir.rules().len(), 2);
        assert_eq!(ir.conditions().len(), 1);
        assert_eq!(ir.breakpoints().len(), 1);
        // The @provider annotation becomes a Directive node.
        assert_eq!(ir.nodes_by_kind(IRKind::Directive).len(), 1);
        assert_eq!(ir.nodes.len(), 5);
    }

    #[test]
    fn test_build_ir_attribute_mapping() {
        let ir = build_ir(ANNOTATED, "file:///t.rtpl", 1);
        let admin = ir.find_node("RULE:ADMIN_TONE").unwrap();
        assert_eq!(admin.meta.category.as_deref(), Some("tone"));
        assert_eq!(admin.meta.priority, Some(10));
        assert_eq!(admin.meta.triggers, vec!["topic:code", "topic:ops"]);
        assert!(!admin.meta.inferred);

        let bp = ir.find_node("BREAKPOINT:BEFORE_ANSWER").unwrap();
        assert_eq!(bp.meta.condition_ref.as_deref(), Some("CONDITION:ADMIN"));
    }

    #[test]
    fn test_build_ir_meta_is_deterministic_annotation_engine() {
        let ir = build_ir(ANNOTATED, "file:///t.rtpl", 1);
        let meta = ir.meta.as_ref().unwrap();
        assert_eq!(meta.engine, ANNOTATION_ENGINE);
        assert_eq!(meta.confidence, 1.0);
        assert!(meta.warnings.is_empty());
    }

    #[test]
    fn test_build_ir_is_byte_identical_across_runs() {
        let a = serde_json::to_string(&build_ir(ANNOTATED, "file:///t.rtpl", 1)).unwrap();
        let b = serde_json::to_string(&build_ir(ANNOTATED, "file:///t.rtpl", 1)).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn test_build_ir_warns_on_dangling_condition_ref() {
        let src = "// @breakpoint BREAKPOINT:X conditionRef=CONDITION:MISSING\nhi";
        let ir = build_ir(src, "file:///t.rtpl", 1);
        let warnings = &ir.meta.as_ref().unwrap().warnings;
        assert!(warnings.iter().any(|w| w.contains("unknown condition")));
    }

    #[test]
    fn test_build_ir_warns_on_duplicate_ids() {
        let src = "// @rule RULE:DUP\n// @rule RULE:DUP\nhi";
        let ir = build_ir(src, "file:///t.rtpl", 1);
        let warnings = &ir.meta.as_ref().unwrap().warnings;
        assert!(warnings.iter().any(|w| w.contains("duplicate node id")));
    }
    #[test]
    fn rejects_a_second_else_in_one_if_block() {
        // Two depth-1 separators make the renderer emit the text after the first
        // while anchoring provenance on the last, so a rule body can appear in the
        // output while coverage reports its rule uncovered. The unreachable-rule
        // soundness argument reasons from guard chains to what renders, so it
        // loses its footing on such a template. Reject it at parse time instead.
        let template = concat!(
            "// @rule RULE:D category=p priority=2\n",
            "D body\n",
            "{% if flag %}\n",
            "x\n",
            "{% else %}\n",
            "// @rule RULE:R category=p priority=1\n",
            "R body\n",
            "{% else %}\n",
            "y\n",
            "{% endif %}\n"
        );
        assert_eq!(duplicate_else_lines(template), vec![7]);
        let err = render(template, &serde_json::json!({"flag": false}))
            .expect_err("a second else must be rejected");
        assert!(matches!(err, TemplateError::ParseError { line: 8, .. }), "got {err:?}");

        // A single else in the same shape stays legal and renders as before.
        let ok = template.replace("{% else %}\ny\n", "");
        assert!(duplicate_else_lines(&ok).is_empty());
        assert!(render(&ok, &serde_json::json!({"flag": false})).is_ok());
    }

}

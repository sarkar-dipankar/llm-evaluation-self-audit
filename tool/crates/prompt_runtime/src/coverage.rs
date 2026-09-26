//! Coverage analysis for structured prompts.
//!
//! Given a prompt's [`PromptIR`] and a set of input contexts, this computes how
//! thoroughly the inputs exercise the prompt's structure. Coverage is reported in
//! two views, per the project's research design:
//!
//! - **Structural** — derived from [`prompt_template::RenderResult::rendered_lines`]:
//!   a node is covered when its source line falls inside a rendered (taken) region.
//!   Deterministic and provider-free.
//! - **Trace** — derived from the execution [`PromptTrace`]: a rule is covered when
//!   it appears in `rules_used` of a non-planning step. This reads back what the
//!   debugger's trace actually recorded, validating it against the structural view.
//!   For the deterministic engine the two coincide by construction; a divergent
//!   *semantic* view would require an answer-level oracle (future work).
//!
//! Three criteria are computed: rule coverage, condition/branch coverage, and
//! rule-pair (interaction) coverage.

use std::collections::{BTreeSet, HashSet};

use prompt_ir::{IRKind, PromptIR, ProviderConfig, TraceStepKind};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{execute_with_details, DebugConfig, RuntimeResult};

/// Per-branch outcome coverage for a single `{% if %}` block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchCoverage {
    /// Absolute, 0-based source line of the `{% if %}`.
    pub condition_line: u32,
    /// The condition expression.
    pub condition: String,
    /// Whether the true (taken) outcome was observed across the inputs.
    pub true_seen: bool,
    /// Whether the false (not-taken) outcome was observed across the inputs.
    pub false_seen: bool,
}

/// A full coverage report for a prompt over a set of input contexts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageReport {
    pub uri: String,
    pub num_inputs: usize,

    // --- Rule coverage (structural view) ---
    pub total_rules: usize,
    pub covered_rules_structural: usize,
    pub rule_coverage_structural: f64,
    pub uncovered_rules_structural: Vec<String>,

    // --- Rule coverage (trace view) ---
    pub covered_rules_trace: usize,
    pub rule_coverage_trace: f64,
    pub uncovered_rules_trace: Vec<String>,

    // --- Condition node coverage (structural) ---
    pub total_conditions: usize,
    pub covered_conditions: usize,
    pub condition_coverage: f64,

    // --- Branch outcome coverage ---
    pub total_branches: usize,
    pub total_branch_outcomes: usize,
    pub covered_branch_outcomes: usize,
    pub branch_coverage: f64,
    pub branches: Vec<BranchCoverage>,

    // --- Rule-pair (interaction) coverage ---
    pub total_rule_pairs: usize,
    pub covered_rule_pairs: usize,
    pub rule_pair_coverage: f64,
}

/// A coverage ratio with the convention that 0/0 == 1.0 (vacuously covered).
fn ratio(num: usize, den: usize) -> f64 {
    if den == 0 {
        1.0
    } else {
        num as f64 / den as f64
    }
}

/// Statically locate every `{% if ... %}` block, returning (0-based line, condition).
/// Mirrors the template engine's own parsing of control directives.
fn scan_if_blocks(template: &str) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    for (i, line) in template.lines().enumerate() {
        if let Some(inner) = line
            .trim()
            .strip_prefix("{%")
            .and_then(|s| s.strip_suffix("%}"))
        {
            if let Some(cond) = inner.trim().strip_prefix("if ") {
                out.push((i as u32, cond.trim().to_string()));
            }
        }
    }
    out
}

/// An ordered rule pair, used as a set key for interaction coverage.
fn pair_key(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

/// Compute coverage of `ir` over `contexts`. Uses `template_only` execution, so no
/// provider or API key is required even for the trace view.
pub async fn compute_coverage(
    ir: &PromptIR,
    template: &str,
    contexts: &[Value],
) -> RuntimeResult<CoverageReport> {
    // An empty context set still exercises the unconditional (top-level) structure.
    let default = [json!({})];
    let contexts: &[Value] = if contexts.is_empty() {
        &default
    } else {
        contexts
    };

    // Rule and condition nodes with their 0-based source lines.
    let rule_lines: Vec<(String, u32)> = ir
        .nodes
        .iter()
        .filter(|n| n.kind == IRKind::Rule)
        .map(|n| (n.id.clone(), n.range.start_line))
        .collect();
    let cond_lines: Vec<(String, u32)> = ir
        .nodes
        .iter()
        .filter(|n| n.kind == IRKind::Condition)
        .map(|n| (n.id.clone(), n.range.start_line))
        .collect();
    let rule_ids: HashSet<&str> = rule_lines.iter().map(|(id, _)| id.as_str()).collect();

    // Branch universe from a static scan (so never-reached branches still count).
    let if_blocks = scan_if_blocks(template);
    let mut branch_outcomes: std::collections::BTreeMap<u32, (String, bool, bool)> = if_blocks
        .iter()
        .map(|(line, cond)| (*line, (cond.clone(), false, false)))
        .collect();

    let mut covered_rules_structural: HashSet<String> = HashSet::new();
    let mut covered_conditions: HashSet<String> = HashSet::new();
    let mut covered_pairs: HashSet<(String, String)> = HashSet::new();
    let mut covered_rules_trace: HashSet<String> = HashSet::new();

    for ctx in contexts {
        let render = prompt_template::render(template, ctx)?;
        let rendered: HashSet<u32> = render.rendered_lines.iter().copied().collect();

        // Structural rule coverage + per-context covered set for pair coverage.
        let mut this_ctx_rules: Vec<String> = Vec::new();
        for (id, line) in &rule_lines {
            if rendered.contains(line) {
                covered_rules_structural.insert(id.clone());
                this_ctx_rules.push(id.clone());
            }
        }
        this_ctx_rules.sort();
        this_ctx_rules.dedup();
        for i in 0..this_ctx_rules.len() {
            for j in (i + 1)..this_ctx_rules.len() {
                covered_pairs.insert(pair_key(&this_ctx_rules[i], &this_ctx_rules[j]));
            }
        }

        // Condition node coverage.
        for (id, line) in &cond_lines {
            if rendered.contains(line) {
                covered_conditions.insert(id.clone());
            }
        }

        // Branch outcome coverage (keyed by the `{% if %}` line).
        for branch in &render.branches_taken {
            if let Some(entry) = branch_outcomes.get_mut(&branch.condition_line) {
                if branch.taken {
                    entry.1 = true;
                } else {
                    entry.2 = true;
                }
            }
        }

        // Trace view: union of rules_used from non-planning execution steps.
        let exec = execute_with_details(
            template,
            ir,
            &ProviderConfig::default(),
            &DebugConfig {
                context: ctx.clone(),
                breakpoints: Vec::new(),
                template_only: true,
                stop_on_entry: false,
            },
        )
        .await?;
        for step in &exec.trace.steps {
            if step.kind == TraceStepKind::Execution {
                for r in &step.rules_used {
                    if rule_ids.contains(r.as_str()) {
                        covered_rules_trace.insert(r.clone());
                    }
                }
            }
        }
    }

    let total_rules = rule_lines.len();
    let total_conditions = cond_lines.len();

    let uncovered_rules_structural: Vec<String> = rule_lines
        .iter()
        .filter(|(id, _)| !covered_rules_structural.contains(id))
        .map(|(id, _)| id.clone())
        .collect();
    let uncovered_rules_trace: Vec<String> = rule_lines
        .iter()
        .filter(|(id, _)| !covered_rules_trace.contains(id))
        .map(|(id, _)| id.clone())
        .collect();

    // Branch outcomes.
    let branches: Vec<BranchCoverage> = branch_outcomes
        .iter()
        .map(|(line, (cond, t, f))| BranchCoverage {
            condition_line: *line,
            condition: cond.clone(),
            true_seen: *t,
            false_seen: *f,
        })
        .collect();
    let total_branches = branches.len();
    let total_branch_outcomes = total_branches * 2;
    let covered_branch_outcomes: usize = branches
        .iter()
        .map(|b| usize::from(b.true_seen) + usize::from(b.false_seen))
        .sum();

    // Rule-pair universe.
    let unique_rule_ids: BTreeSet<&str> = rule_lines.iter().map(|(id, _)| id.as_str()).collect();
    let r = unique_rule_ids.len();
    let total_rule_pairs = r * r.saturating_sub(1) / 2;

    Ok(CoverageReport {
        uri: ir.uri.clone(),
        num_inputs: contexts.len(),
        total_rules,
        covered_rules_structural: covered_rules_structural.len(),
        rule_coverage_structural: ratio(covered_rules_structural.len(), total_rules),
        uncovered_rules_structural,
        covered_rules_trace: covered_rules_trace.len(),
        rule_coverage_trace: ratio(covered_rules_trace.len(), total_rules),
        uncovered_rules_trace,
        total_conditions,
        covered_conditions: covered_conditions.len(),
        condition_coverage: ratio(covered_conditions.len(), total_conditions),
        total_branches,
        total_branch_outcomes,
        covered_branch_outcomes,
        branch_coverage: ratio(covered_branch_outcomes, total_branch_outcomes),
        branches,
        total_rule_pairs,
        covered_rule_pairs: covered_pairs.len(),
        rule_pair_coverage: ratio(covered_pairs.len(), total_rule_pairs),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use prompt_template::build_ir;

    // 0 // @rule RULE:TOP
    // 1 Always present.
    // 2 {% if admin %}
    // 3 // @condition CONDITION:ADMIN
    // 4 // @rule RULE:ADMIN
    // 5 admin text
    // 6 {% else %}
    // 7 // @rule RULE:USER
    // 8 user text
    // 9 {% endif %}
    const TEMPLATE: &str = "// @rule RULE:TOP\nAlways present.\n{% if admin %}\n// @condition CONDITION:ADMIN\n// @rule RULE:ADMIN\nadmin text\n{% else %}\n// @rule RULE:USER\nuser text\n{% endif %}";

    fn ir() -> PromptIR {
        build_ir(TEMPLATE, "file:///t.rtpl", 1)
    }

    #[tokio::test]
    async fn test_full_coverage_with_both_inputs() {
        let report = compute_coverage(
            &ir(),
            TEMPLATE,
            &[json!({"admin": true}), json!({"admin": false})],
        )
        .await
        .unwrap();

        assert_eq!(report.total_rules, 3);
        assert_eq!(report.rule_coverage_structural, 1.0);
        assert_eq!(report.rule_coverage_trace, 1.0);
        // One branch, both outcomes observed.
        assert_eq!(report.total_branches, 1);
        assert_eq!(report.branch_coverage, 1.0);
        // CONDITION:ADMIN only renders on the admin path, which we exercised.
        assert_eq!(report.condition_coverage, 1.0);
    }

    #[tokio::test]
    async fn test_partial_coverage_single_input() {
        let report = compute_coverage(&ir(), TEMPLATE, &[json!({"admin": true})])
            .await
            .unwrap();

        // RULE:TOP + RULE:ADMIN covered, RULE:USER not.
        assert_eq!(report.covered_rules_structural, 2);
        assert!((report.rule_coverage_structural - 2.0 / 3.0).abs() < 1e-9);
        assert_eq!(report.uncovered_rules_structural, vec!["RULE:USER"]);
        // Only the true outcome of the branch was seen.
        assert_eq!(report.covered_branch_outcomes, 1);
        assert_eq!(report.branch_coverage, 0.5);
        // Trace view agrees with structural for the deterministic engine.
        assert_eq!(report.covered_rules_trace, 2);
    }

    #[tokio::test]
    async fn test_rule_pair_coverage() {
        // ADMIN and USER live in mutually exclusive branches, so they never co-occur.
        // TOP co-occurs with whichever branch rule rendered.
        let report = compute_coverage(
            &ir(),
            TEMPLATE,
            &[json!({"admin": true}), json!({"admin": false})],
        )
        .await
        .unwrap();

        // 3 rules → C(3,2) = 3 possible pairs.
        assert_eq!(report.total_rule_pairs, 3);
        // Covered: (TOP,ADMIN) and (TOP,USER). Never (ADMIN,USER).
        assert_eq!(report.covered_rule_pairs, 2);
        assert!((report.rule_pair_coverage - 2.0 / 3.0).abs() < 1e-9);
    }
}

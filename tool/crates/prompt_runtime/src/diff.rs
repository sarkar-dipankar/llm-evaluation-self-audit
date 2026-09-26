//! Behavior signatures, trace diffing, and kill-detection oracles.
//!
//! A [`BehaviorSignature`] is the deterministic, observable behavior of a prompt on
//! one input context: the rendered output plus the structural facts (which rules
//! rendered, which branch outcomes occurred). Comparing signatures is how the
//! mutation engine decides whether a mutant is *killed* by a test input.
//!
//! Two oracle styles are provided:
//! - [`TraceOracle`] / [`OutputOracle`] — deterministic, provider-free.
//! - [`llm_judge_killed`] — an answer-level oracle that asks a model whether two
//!   outputs differ meaningfully (used only when a provider is configured).

use std::collections::{BTreeMap, BTreeSet};

use prompt_ir::{IRKind, PromptIR, PromptTrace};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::provider::ProviderClient;
use crate::RuntimeResult;

/// The observable behavior of a prompt on a single input context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BehaviorSignature {
    /// The fully rendered prompt text.
    pub output: String,
    /// Rules whose source line fell inside a rendered region.
    pub covered_rules: BTreeSet<String>,
    /// Covered rules that actually "win" after conflict resolution (see
    /// [`resolve_applied`]). Including this makes rule priority/category/order
    /// behaviorally observable, so metadata mutations are no longer equivalent.
    pub applied_rules: BTreeSet<String>,
    /// Branch outcomes observed: `{% if %}` line (0-based) → taken.
    pub branch_outcomes: BTreeMap<u32, bool>,
}

/// Resolve which covered rules are *applied* vs *shadowed*.
///
/// Rules carrying the same `category` conflict; the one with the highest `priority`
/// (ties broken by earliest source line) wins and is applied, the rest are shadowed.
/// Rules without a category do not conflict and are always applied. This models
/// prompt rule precedence and is what makes `ChangePriority` / `ChangeCategory` /
/// (order-breaking) `SwapRules` mutations observable.
pub fn resolve_applied(ir: &PromptIR, covered: &BTreeSet<String>) -> BTreeSet<String> {
    use std::collections::BTreeMap as Map;
    let mut by_cat: Map<String, Vec<(i32, u32, String)>> = Map::new();
    let mut applied = BTreeSet::new();

    for node in &ir.nodes {
        if node.kind != IRKind::Rule || !covered.contains(&node.id) {
            continue;
        }
        match &node.meta.category {
            Some(cat) => by_cat.entry(cat.clone()).or_default().push((
                node.meta.priority.unwrap_or(0),
                node.range.start_line,
                node.id.clone(),
            )),
            None => {
                applied.insert(node.id.clone());
            }
        }
    }

    for (_cat, mut rules) in by_cat {
        // Highest priority wins; ties broken by earliest source line.
        rules.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        if let Some((_, _, id)) = rules.first() {
            applied.insert(id.clone());
        }
    }
    applied
}

/// Compute the behavior signature of `template` (with structure `ir`) on `context`.
pub fn behavior_signature(
    ir: &PromptIR,
    template: &str,
    context: &Value,
) -> RuntimeResult<BehaviorSignature> {
    let render = prompt_template::render(template, context)?;
    let rendered: BTreeSet<u32> = render.rendered_lines.iter().copied().collect();

    let covered_rules: BTreeSet<String> = ir
        .nodes
        .iter()
        .filter(|n| n.kind == IRKind::Rule && rendered.contains(&n.range.start_line))
        .map(|n| n.id.clone())
        .collect();

    let applied_rules = resolve_applied(ir, &covered_rules);

    let branch_outcomes = render
        .branches_taken
        .iter()
        .map(|b| (b.condition_line, b.taken))
        .collect();

    Ok(BehaviorSignature {
        output: render.output,
        covered_rules,
        applied_rules,
        branch_outcomes,
    })
}

/// Decides whether a mutant is killed by a single input, given the original and
/// mutant behavior on that input.
pub trait Oracle {
    fn killed(&self, original: &BehaviorSignature, mutant: &BehaviorSignature) -> bool;
}

/// Kills a mutant when any observable behavior differs (output, covered rules, or
/// branch outcomes). This is the default for deterministic mutation testing: a
/// mutant survives only if no test input can distinguish it.
#[derive(Debug, Default, Clone, Copy)]
pub struct TraceOracle;

impl Oracle for TraceOracle {
    fn killed(&self, original: &BehaviorSignature, mutant: &BehaviorSignature) -> bool {
        original != mutant
    }
}

/// Kills a mutant only when the rendered output text differs.
#[derive(Debug, Default, Clone, Copy)]
pub struct OutputOracle;

impl Oracle for OutputOracle {
    fn killed(&self, original: &BehaviorSignature, mutant: &BehaviorSignature) -> bool {
        original.output != mutant.output
    }
}

/// A structural diff between two execution traces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceDiff {
    pub rules_only_in_original: Vec<String>,
    pub rules_only_in_mutant: Vec<String>,
    pub step_count_original: usize,
    pub step_count_mutant: usize,
}

impl TraceDiff {
    /// Whether the two traces differ in any tracked dimension.
    pub fn is_different(&self) -> bool {
        !self.rules_only_in_original.is_empty()
            || !self.rules_only_in_mutant.is_empty()
            || self.step_count_original != self.step_count_mutant
    }
}

/// Diff the rules used and step counts of two traces.
pub fn diff_traces(original: &PromptTrace, mutant: &PromptTrace) -> TraceDiff {
    let orig_rules: BTreeSet<&str> = original.rules_used().into_iter().collect();
    let mut_rules: BTreeSet<&str> = mutant.rules_used().into_iter().collect();

    TraceDiff {
        rules_only_in_original: orig_rules
            .difference(&mut_rules)
            .map(|s| s.to_string())
            .collect(),
        rules_only_in_mutant: mut_rules
            .difference(&orig_rules)
            .map(|s| s.to_string())
            .collect(),
        step_count_original: original.steps.len(),
        step_count_mutant: mutant.steps.len(),
    }
}

/// Answer-level oracle: ask a model whether two rendered prompts would produce
/// meaningfully different behavior. Returns true (killed) when the judge says they
/// differ. Requires a configured provider; used for the semantic view, not the
/// deterministic mutation score.
pub async fn llm_judge_killed(
    client: &dyn ProviderClient,
    original_output: &str,
    mutant_output: &str,
) -> RuntimeResult<bool> {
    let prompt = format!(
        "You compare two LLM prompts. Answer with exactly YES if they would lead an \
assistant to behave differently, or NO if the difference is cosmetic and behavior \
would be equivalent.\n\n--- PROMPT A ---\n{original_output}\n\n--- PROMPT B ---\n{mutant_output}\n\nAnswer (YES/NO):"
    );
    let answer = client.complete(&prompt).await?;
    Ok(answer.trim().to_uppercase().starts_with("YES"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use prompt_template::build_ir;
    use serde_json::json;

    const T: &str = "// @rule RULE:A\nhello\n{% if admin %}\n// @rule RULE:B\nadmin\n{% endif %}";

    #[test]
    fn test_signature_changes_with_branch() {
        let ir = build_ir(T, "file:///t", 1);
        let on = behavior_signature(&ir, T, &json!({"admin": true})).unwrap();
        let off = behavior_signature(&ir, T, &json!({"admin": false})).unwrap();
        assert!(on.covered_rules.contains("RULE:B"));
        assert!(!off.covered_rules.contains("RULE:B"));
        assert!(TraceOracle.killed(&on, &off));
        assert!(OutputOracle.killed(&on, &off));
    }

    #[test]
    fn test_resolve_applied_shadows_lower_priority_same_category() {
        // Two style rules; the higher-priority one wins, the other is shadowed.
        let t = "// @rule RULE:LOW category=style priority=1\n// @rule RULE:HIGH category=style priority=9\nhi";
        let ir = build_ir(t, "file:///t", 1);
        let covered: BTreeSet<String> = ["RULE:LOW", "RULE:HIGH"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let applied = resolve_applied(&ir, &covered);
        assert!(applied.contains("RULE:HIGH"));
        assert!(!applied.contains("RULE:LOW"));
    }

    #[test]
    fn test_uncategorized_rules_never_shadowed() {
        let t = "// @rule RULE:A\n// @rule RULE:B\nhi";
        let ir = build_ir(t, "file:///t", 1);
        let covered: BTreeSet<String> =
            ["RULE:A", "RULE:B"].iter().map(|s| s.to_string()).collect();
        let applied = resolve_applied(&ir, &covered);
        assert_eq!(applied.len(), 2);
    }

    #[test]
    fn test_identical_signatures_not_killed() {
        let ir = build_ir(T, "file:///t", 1);
        let a = behavior_signature(&ir, T, &json!({"admin": true})).unwrap();
        let b = behavior_signature(&ir, T, &json!({"admin": true})).unwrap();
        assert!(!TraceOracle.killed(&a, &b));
    }
}

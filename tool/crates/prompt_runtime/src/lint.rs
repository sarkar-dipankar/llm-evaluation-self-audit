//! Deterministic static analysis over the prompt IR.
//!
//! Currently detects **unreachable (always-shadowed) rules**: a categorized rule that
//! can never win its category because a same-category rule with strictly higher
//! priority is rendered in every context where it is. Provider-free and reproducible,
//! complementing the LLM linter's heuristic conflict detection.
//!
//! Soundness: rule `D` dominates rule `R` only when `D`'s branch-guard chain is a
//! *prefix* of `R`'s — i.e. `D` sits in an enclosing-or-equal region, so whenever `R`
//! renders, `D` does too. This avoids false positives across mutually exclusive
//! branches and is intentionally conservative (strictly-higher priority only, so
//! equal-priority peers are not flagged).
//!
//! That argument only holds if the guard chains this analysis reasons about are the
//! ones the renderer actually enforces. They are: [`prompt_template::guard_chains`]
//! is computed from the renderer's own block scanners, so a tag the renderer treats
//! as a branch separator is one this analysis sees too, and vice versa. Deriving the
//! chains separately here is what previously let the analyser report a rule as
//! unreachable in a template the renderer in fact rendered it in (see
//! `test_embedded_else_is_a_real_separator`).

use prompt_ir::{IRKind, PromptDiagnostic, PromptIR};
use prompt_template::{guard_chains, Guard};

/// True when `outer` is a prefix of `inner` (outer encloses or equals inner's region).
fn is_prefix(outer: &[Guard], inner: &[Guard]) -> bool {
    outer.len() <= inner.len() && inner[..outer.len()] == *outer
}

/// Detect unreachable (always-shadowed) rules.
pub fn unreachable_rules(ir: &PromptIR, template: &str) -> Vec<PromptDiagnostic> {
    let chains = guard_chains(template);
    let rules: Vec<(&prompt_ir::IRNode, Vec<Guard>)> = ir
        .nodes
        .iter()
        .filter(|n| n.kind == IRKind::Rule)
        .map(|n| {
            let guards = chains
                .get(n.range.start_line as usize)
                .cloned()
                .unwrap_or_default();
            (n, guards)
        })
        .collect();

    let mut diags = Vec::new();
    for (node, guards) in &rules {
        let Some(cat) = &node.meta.category else {
            continue; // uncategorized rules never conflict
        };
        let prio = node.meta.priority.unwrap_or(0);

        if let Some((dom, _)) = rules.iter().find(|(other, og)| {
            other.id != node.id
                && other.meta.category.as_deref() == Some(cat.as_str())
                && other.meta.priority.unwrap_or(0) > prio
                && is_prefix(og, guards)
        }) {
            diags.push(
                PromptDiagnostic::warning(
                    format!(
                        "Rule {} is unreachable: always shadowed by higher-priority rule {} in category '{}'",
                        node.id, dom.id, cat
                    ),
                    node.range,
                )
                .with_code("prompt/unreachable-rule"),
            );
        }
    }
    diags
}

#[cfg(test)]
mod tests {
    use super::*;
    use prompt_template::build_ir;

    #[test]
    fn test_flags_lower_priority_same_region() {
        let t = "// @rule RULE:LOW category=style priority=1\n// @rule RULE:HIGH category=style priority=9\nhi";
        let d = unreachable_rules(&build_ir(t, "f", 1), t);
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("RULE:LOW"));
        assert_eq!(d[0].code.as_deref(), Some("prompt/unreachable-rule"));
    }

    #[test]
    fn test_no_flag_across_exclusive_branches() {
        // HIGH is in the if-side, LOW in the else-side: never co-render, not shadowed.
        let t = "{% if x %}\n// @rule RULE:HIGH category=style priority=9\n{% else %}\n// @rule RULE:LOW category=style priority=1\n{% endif %}";
        assert!(unreachable_rules(&build_ir(t, "f", 1), t).is_empty());
    }

    #[test]
    fn test_no_flag_equal_priority_or_uncategorized() {
        let eq = "// @rule RULE:A category=style\n// @rule RULE:B category=style\nhi";
        assert!(unreachable_rules(&build_ir(eq, "f", 1), eq).is_empty());
        let uncat = "// @rule RULE:A priority=1\n// @rule RULE:B priority=9\nhi";
        assert!(unreachable_rules(&build_ir(uncat, "f", 1), uncat).is_empty());
    }

    /// Regression: an `{% else %}` embedded in prose is a real branch separator to
    /// the renderer, so the analyser must see it too.
    ///
    /// With `flag = false` the renderer emits `RULE:R` and not `RULE:D`, so `R` is
    /// not shadowed. The analyser used to recognise a control tag only when it was
    /// alone on its line, put both rules in the same region, and report `R`
    /// unreachable: a false positive, and a hole in the soundness argument above.
    #[test]
    fn test_embedded_else_is_a_real_separator() {
        let t = "{% if flag %}\n\
                 // @rule RULE:D category=p priority=2\n\
                 D body\n\
                 literal {% else %} marker\n\
                 // @rule RULE:R category=p priority=1\n\
                 R body\n\
                 {% endif %}";

        // The renderer really does treat the embedded tag as the separator.
        let out = prompt_template::render(t, &serde_json::json!({ "flag": false }))
            .unwrap()
            .output;
        assert!(out.contains("R body"), "renderer should emit R: {out:?}");
        assert!(!out.contains("D body"), "renderer should not emit D: {out:?}");

        // So the analyser must not claim R is always shadowed by D.
        let d = unreachable_rules(&build_ir(t, "f", 1), t);
        assert!(d.is_empty(), "expected no diagnostics, got {d:?}");

        // The stricter reading (a control tag must be alone on its line) flags the
        // line, for callers that prefer to reject such templates outright.
        assert_eq!(prompt_template::embedded_control_tags(t), vec![3]);
    }

    #[test]
    fn test_flags_nested_rule_dominated_by_outer() {
        // Outer top-level HIGH dominates a nested LOW of the same category.
        let t = "// @rule RULE:HIGH category=style priority=9\n{% if x %}\n// @rule RULE:LOW category=style priority=1\n{% endif %}";
        let d = unreachable_rules(&build_ir(t, "f", 1), t);
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("RULE:LOW"));
    }
}

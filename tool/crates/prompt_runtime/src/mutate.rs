//! Mutation engine for structured prompts.
//!
//! Defines mutation operators over a prompt's annotations and template control
//! flow, generates one mutant per applicable site, and scores a test suite (a set
//! of input contexts) by how many mutants it *kills* via an [`Oracle`].
//!
//! The same mutants double as **seeded defects** for evaluating the LLM linter
//! (RQ3): each mutant is a known, localized fault with a label (its operator).

use std::collections::BTreeMap;

use prompt_ir::PromptIR;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::diff::{behavior_signature, Oracle, TraceOracle};
use crate::RuntimeResult;

/// A mutation operator. Each names a localized, behavior-relevant fault class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MutationOperator {
    /// Delete a `// @rule` annotation.
    DropRule,
    /// Delete a `// @condition` annotation.
    DropCondition,
    /// Negate a `{% if %}` condition.
    NegateCondition,
    /// Swap two consecutive `// @rule` annotations (reordering).
    SwapRules,
    /// Change a rule's `priority=` attribute value.
    ChangePriority,
    /// Change an annotation's `category=` attribute value.
    ChangeCategory,
}

impl MutationOperator {
    pub fn as_str(&self) -> &'static str {
        match self {
            MutationOperator::DropRule => "DropRule",
            MutationOperator::DropCondition => "DropCondition",
            MutationOperator::NegateCondition => "NegateCondition",
            MutationOperator::SwapRules => "SwapRules",
            MutationOperator::ChangePriority => "ChangePriority",
            MutationOperator::ChangeCategory => "ChangeCategory",
        }
    }
}

/// A single generated mutant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mutant {
    /// Stable id, e.g. `NegateCondition@L5`.
    pub id: String,
    pub operator: MutationOperator,
    pub description: String,
    /// 0-based source line the mutation targets.
    pub line: u32,
    /// True when the edit touches annotation comment lines only.
    ///
    /// Annotations are stripped before the prompt reaches the model, so a
    /// metadata-only mutant renders byte-identically to the original: it can be
    /// killed by the structural [`TraceOracle`] but never by an output-only
    /// oracle. The two classes answer different questions — a body-carrying
    /// mutant is a fault in the text the model reads, a metadata-only mutant is a
    /// fault in the toolchain's view of the prompt — so they are tagged rather
    /// than mixed, and reported separately in [`OperatorStats::metadata_only`].
    ///
    /// This is also how a `DropRule` site with no body is handled. `block_extent`
    /// stops at the next annotation or control tag, so a rule annotation
    /// immediately followed by another annotation or by `{% if %}` labels no body
    /// at all and its extent is a single line. Dropping it removes a label and
    /// nothing else. Such sites are still generated (they are real prompt edits,
    /// and the structural oracle does distinguish them) but are tagged here so
    /// callers never have to infer the distinction from the description string.
    pub metadata_only: bool,
    /// The full mutated prompt source.
    pub source: String,
}

/// Negate a template condition expression.
fn negate_condition(cond: &str) -> String {
    let c = cond.trim();
    if let Some(rest) = c.strip_prefix("not ") {
        return rest.trim().to_string();
    }
    if let Some(rest) = c.strip_prefix('!') {
        return rest.trim().to_string();
    }
    if c.contains("==") {
        return c.replacen("==", "!=", 1);
    }
    if c.contains("!=") {
        return c.replacen("!=", "==", 1);
    }
    format!("not {c}")
}

/// Parse an `// @kind ID attr=val ...` annotation line into its leading whitespace,
/// kind keyword, and the remainder, if it is an annotation.
fn annotation_parts(line: &str) -> Option<(&str, String, &str)> {
    let indent_len = line.len() - line.trim_start().len();
    let (indent, rest) = line.split_at(indent_len);
    let rest = rest.strip_prefix("// @")?;
    let kind = rest.split_whitespace().next()?.to_lowercase();
    Some((indent, kind, rest))
}

fn is_rule_line(line: &str) -> bool {
    matches!(annotation_parts(line), Some((_, k, _)) if k.starts_with("rule"))
}

fn is_condition_line(line: &str) -> bool {
    matches!(annotation_parts(line), Some((_, k, _)) if k.starts_with("condition"))
}

/// Split a template into lines, remembering whether it was newline-terminated.
///
/// The flag is what makes line edits newline-consistent. `str::lines` drops the
/// final terminator, so rebuilding with a plain `join` silently rewrites a
/// newline-terminated template into an unterminated one. That is invisible for a
/// mutation in the middle of the file but not for one that removes the last line:
/// the original still renders its final (blank) line while the mutant does not,
/// and the rendered outputs then differ by a trailing newline that no prompt
/// author ever wrote. `DropCondition` scored exactly one output-oracle kill
/// across the corpus that way. Carrying the flag through [`join`] keeps
/// `join(split_lines(t)) == t` for every template.
fn split_lines(template: &str) -> (Vec<String>, bool) {
    (
        template.lines().map(|s| s.to_string()).collect(),
        template.ends_with('\n'),
    )
}

/// Rebuild a source string from lines, restoring the original trailing newline.
fn join(lines: &[String], trailing_newline: bool) -> String {
    let mut s = lines.join("\n");
    if trailing_newline && !lines.is_empty() {
        s.push('\n');
    }
    s
}

/// The span of lines owned by the annotation at `start`: the annotation line
/// itself plus the instruction body that follows it, stopping before the next
/// annotation, the next control-flow tag, or the end of the template. Trailing
/// blank lines are left out so that removing a block does not also collapse the
/// spacing around it.
///
/// This matters because annotations are comments and are stripped at render
/// time. An operator that edits only the annotation line cannot change the text
/// the model receives, so mutants generated that way are equivalent under any
/// output-based oracle. Operators that delete or move a rule must carry the
/// body with them to correspond to a fault a prompt author could actually make.
fn block_extent(lines: &[String], start: usize) -> std::ops::Range<usize> {
    let mut end = start + 1;
    while end < lines.len() {
        let is_annotation = lines[end].trim_start().starts_with("// @");
        // Uses the renderer's own dispatch rule so that a line the renderer would
        // treat as control flow always ends the body, and a tag embedded in prose
        // (which the renderer renders as text) never does.
        if is_annotation || prompt_template::is_control_line(&lines[end]) {
            break;
        }
        end += 1;
    }
    while end > start + 1 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    start..end
}

/// Swap two non-overlapping line blocks, `a` occurring before `b`, preserving
/// everything between and around them.
fn swap_blocks(
    lines: &[String],
    a: std::ops::Range<usize>,
    b: std::ops::Range<usize>,
) -> Vec<String> {
    debug_assert!(a.end <= b.start, "blocks must not overlap");
    let mut out = Vec::with_capacity(lines.len());
    out.extend_from_slice(&lines[..a.start]);
    out.extend_from_slice(&lines[b.start..b.end]);
    out.extend_from_slice(&lines[a.end..b.start]);
    out.extend_from_slice(&lines[a.start..a.end]);
    out.extend_from_slice(&lines[b.end..]);
    out
}

/// Replace the `key=value` attribute on an annotation line, or append it if absent.
fn set_attribute(line: &str, key: &str, value: &str) -> String {
    let mut tokens: Vec<String> = line.split_whitespace().map(|s| s.to_string()).collect();
    let mut replaced = false;
    for tok in tokens.iter_mut() {
        if tok.starts_with(&format!("{key}=")) {
            *tok = format!("{key}={value}");
            replaced = true;
        }
    }
    if !replaced {
        tokens.push(format!("{key}={value}"));
    }
    let indent_len = line.len() - line.trim_start().len();
    format!("{}{}", &line[..indent_len], tokens.join(" "))
}

/// Generate all mutants for a prompt. One mutant per applicable operator site.
pub fn generate_mutants(template: &str) -> Vec<Mutant> {
    let (lines, trailing_newline) = split_lines(template);
    let mut mutants = Vec::new();

    let mut push = |op: MutationOperator,
                    line: u32,
                    desc: String,
                    metadata_only: bool,
                    new_lines: Vec<String>| {
        mutants.push(Mutant {
            id: format!("{}@L{}", op.as_str(), line + 1),
            operator: op,
            description: desc,
            line,
            metadata_only,
            source: join(&new_lines, trailing_newline),
        });
    };

    // Per-line operators.
    for (i, line) in lines.iter().enumerate() {
        let li = i as u32;

        if is_rule_line(line) {
            // DropRule: remove the rule annotation *and* the instruction body it
            // labels, which is the fault an author makes when deleting a rule.
            let span = block_extent(&lines, i);
            let mut m = lines.clone();
            m.drain(span.clone());
            // An extent of one line means the annotation labels no body, so this
            // drop removes metadata and nothing the model reads. See
            // `Mutant::metadata_only` for why such sites are tagged, not skipped.
            let metadata_only = span.len() == 1;
            push(
                MutationOperator::DropRule,
                li,
                format!(
                    "drop rule and its body ({} line(s)): {}",
                    span.len(),
                    line.trim()
                ),
                metadata_only,
                m,
            );

            // ChangePriority (only if a priority attribute exists)
            if line.contains("priority=") {
                let mut m = lines.clone();
                let cur = line
                    .split_whitespace()
                    .find_map(|t| t.strip_prefix("priority="))
                    .and_then(|v| v.parse::<i64>().ok())
                    .unwrap_or(0);
                m[i] = set_attribute(line, "priority", &(cur + 1000).to_string());
                push(
                    MutationOperator::ChangePriority,
                    li,
                    "bump rule priority by 1000".to_string(),
                    true,
                    m,
                );
            }
        }

        if is_condition_line(line) {
            let mut m = lines.clone();
            m.remove(i);
            push(
                MutationOperator::DropCondition,
                li,
                format!("drop condition annotation: {}", line.trim()),
                true,
                m,
            );
        }

        if annotation_parts(line).is_some() && line.contains("category=") {
            let mut m = lines.clone();
            m[i] = set_attribute(line, "category", "__mutated__");
            push(
                MutationOperator::ChangeCategory,
                li,
                "change annotation category".to_string(),
                true,
                m,
            );
        }

        // NegateCondition on `{% if COND %}`
        if let Some(inner) = line
            .trim()
            .strip_prefix("{%")
            .and_then(|s| s.strip_suffix("%}"))
        {
            if let Some(cond) = inner.trim().strip_prefix("if ") {
                let indent_len = line.len() - line.trim_start().len();
                let mut m = lines.clone();
                m[i] = format!(
                    "{}{{% if {} %}}",
                    &line[..indent_len],
                    negate_condition(cond)
                );
                push(
                    MutationOperator::NegateCondition,
                    li,
                    format!("negate condition: {}", cond.trim()),
                    false,
                    m,
                );
            }
        }
    }

    // SwapRules: exchange each consecutive pair of rule blocks, annotation and
    // instruction body together, so the order the model reads the instructions
    // in actually changes. Swapping the annotation lines alone would only
    // relabel the bodies and leave the rendered prompt untouched.
    //
    // The pair must sit in the same control-flow region. Two rules under
    // different guard chains do not both render under the same conditions, so
    // exchanging them moves an instruction *into* or *out of* a branch or a loop
    // body. That is a guard-changing edit — it adds and removes text rather than
    // reordering it — and attributing its kills to a reordering operator
    // overstates what reordering alone is worth. Equal chains give exactly the
    // property the operator claims: both rules render together, in the swapped
    // order, in every context that renders either.
    let chains = prompt_template::guard_chains(template);
    let rule_indices: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| is_rule_line(l))
        .map(|(i, _)| i)
        .collect();
    for w in rule_indices.windows(2) {
        let (a, b) = (w[0], w[1]);
        if chains.get(a) != chains.get(b) {
            continue;
        }
        let (ra, rb) = (block_extent(&lines, a), block_extent(&lines, b));
        // Adjacent rules can share a boundary; overlapping spans cannot be swapped.
        if ra.end > rb.start {
            continue;
        }
        // Two bodiless annotations exchange labels only; nothing rendered moves.
        let metadata_only = ra.len() == 1 && rb.len() == 1;
        let m = swap_blocks(&lines, ra, rb);
        mutants.push(Mutant {
            id: format!("SwapRules@L{}-{}", a + 1, b + 1),
            operator: MutationOperator::SwapRules,
            description: format!("swap rule blocks at lines {} and {}", a + 1, b + 1),
            line: a as u32,
            metadata_only,
            source: join(&m, trailing_newline),
        });
    }

    mutants
}

/// Per-operator kill statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OperatorStats {
    pub total: usize,
    pub killed: usize,
    /// How many of `total` edit annotation comments only and therefore cannot
    /// change the rendered prompt (see [`Mutant::metadata_only`]).
    pub metadata_only: usize,
}

/// The status of a single mutant under a test suite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutantStatus {
    pub id: String,
    pub operator: MutationOperator,
    pub description: String,
    pub line: u32,
    /// See [`Mutant::metadata_only`].
    pub metadata_only: bool,
    pub killed: bool,
}

/// Result of running a test suite against all mutants.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutationReport {
    pub uri: String,
    pub num_inputs: usize,
    pub total_mutants: usize,
    pub killed: usize,
    pub survived: usize,
    pub mutation_score: f64,
    pub by_operator: BTreeMap<String, OperatorStats>,
    /// Mutants the suite failed to kill (coverage gaps or likely-equivalent).
    pub survivors: Vec<MutantStatus>,
}

/// Run mutation testing: generate mutants and score how many the `contexts` kill,
/// using the given oracle. Deterministic and provider-free with [`TraceOracle`].
pub fn run_mutation_testing(
    original_ir: &PromptIR,
    template: &str,
    contexts: &[Value],
    oracle: &dyn Oracle,
) -> RuntimeResult<MutationReport> {
    let default = [json!({})];
    let contexts: &[Value] = if contexts.is_empty() {
        &default
    } else {
        contexts
    };

    // Baseline behavior of the original on each context.
    let baseline: Vec<_> = contexts
        .iter()
        .map(|c| behavior_signature(original_ir, template, c))
        .collect::<RuntimeResult<_>>()?;

    let mutants = generate_mutants(template);
    let mut by_operator: BTreeMap<String, OperatorStats> = BTreeMap::new();
    let mut survivors = Vec::new();
    let mut killed_count = 0;

    for mutant in &mutants {
        let mutant_ir = prompt_template::build_ir(&mutant.source, &original_ir.uri, 1);
        let mut killed = false;
        for (i, ctx) in contexts.iter().enumerate() {
            // A render error on the mutant counts as a kill (the fault is observable).
            match behavior_signature(&mutant_ir, &mutant.source, ctx) {
                Ok(sig) => {
                    if oracle.killed(&baseline[i], &sig) {
                        killed = true;
                        break;
                    }
                }
                Err(_) => {
                    killed = true;
                    break;
                }
            }
        }

        let stats = by_operator
            .entry(mutant.operator.as_str().to_string())
            .or_default();
        stats.total += 1;
        if mutant.metadata_only {
            stats.metadata_only += 1;
        }
        if killed {
            stats.killed += 1;
            killed_count += 1;
        } else {
            survivors.push(MutantStatus {
                id: mutant.id.clone(),
                operator: mutant.operator,
                description: mutant.description.clone(),
                line: mutant.line,
                metadata_only: mutant.metadata_only,
                killed: false,
            });
        }
    }

    let total = mutants.len();
    let mutation_score = if total == 0 {
        1.0
    } else {
        killed_count as f64 / total as f64
    };

    Ok(MutationReport {
        uri: original_ir.uri.clone(),
        num_inputs: contexts.len(),
        total_mutants: total,
        killed: killed_count,
        survived: total - killed_count,
        mutation_score,
        by_operator,
        survivors,
    })
}

/// Convenience: run mutation testing with the default deterministic oracle.
pub fn run_mutation_testing_default(
    original_ir: &PromptIR,
    template: &str,
    contexts: &[Value],
) -> RuntimeResult<MutationReport> {
    run_mutation_testing(original_ir, template, contexts, &TraceOracle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use prompt_template::build_ir;
    use serde_json::json;

    // Two top-level rules (a same-region pair, so `SwapRules` applies), then a
    // guarded rule in each arm of an if/else (pairs that straddle a branch, so
    // `SwapRules` does not).
    const T: &str = "// @rule RULE:A category=style\nhello\n// @rule RULE:A2 category=tone\nbe brief\n{% if admin %}\n// @condition CONDITION:X\n// @rule RULE:B priority=5\nadmin only\n{% else %}\n// @rule RULE:C\nguest\n{% endif %}";

    #[test]
    fn test_negate_condition_helper() {
        assert_eq!(negate_condition("admin"), "not admin");
        assert_eq!(negate_condition("not admin"), "admin");
        assert_eq!(negate_condition("a == b"), "a != b");
        assert_eq!(negate_condition("a != b"), "a == b");
    }

    #[test]
    fn test_generate_mutants_covers_operators() {
        let mutants = generate_mutants(T);
        let ops: std::collections::BTreeSet<_> =
            mutants.iter().map(|m| m.operator.as_str()).collect();
        assert!(ops.contains("DropRule"));
        assert!(ops.contains("DropCondition"));
        assert!(ops.contains("NegateCondition"));
        assert!(ops.contains("ChangePriority"));
        assert!(ops.contains("ChangeCategory"));
        assert!(ops.contains("SwapRules"));
        // Every mutant must itself be a parseable prompt (build_ir never panics).
        for m in &mutants {
            let _ = build_ir(&m.source, "file:///m", 1);
        }
    }

    #[test]
    fn test_oracle_ablation_delta_on_annotation_only_mutation() {
        use crate::diff::OutputOracle;
        // Two top-level rules; dropping one leaves the rendered prompt unchanged
        // (annotations are stripped from output) but changes the covered-rule set.
        let t = "// @rule RULE:A\n// @rule RULE:B\nhello";
        let ir = build_ir(t, "file:///t", 1);
        let ctxs = [json!({})];
        let trace = run_mutation_testing(&ir, t, &ctxs, &TraceOracle).unwrap();
        let output = run_mutation_testing(&ir, t, &ctxs, &OutputOracle).unwrap();
        // The structural oracle kills DropRule mutants the output oracle cannot see.
        assert!(
            trace.killed > output.killed,
            "trace={} output={}",
            trace.killed,
            output.killed
        );
    }

    #[test]
    fn test_change_priority_now_observable() {
        // Two same-category rules: LOW(1) is shadowed by HIGH(9). Bumping LOW's
        // priority makes it the category winner, changing the applied-rule set, so a
        // ChangePriority mutant is now killed (was equivalent before resolution).
        let t = "// @rule RULE:LOW category=style priority=1\n// @rule RULE:HIGH category=style priority=9\nhi";
        let ir = build_ir(t, "file:///t", 1);
        let report = run_mutation_testing_default(&ir, t, &[json!({})]).unwrap();
        let cp = report
            .by_operator
            .get("ChangePriority")
            .expect("ChangePriority op");
        assert!(
            cp.killed >= 1,
            "expected >=1 ChangePriority kill, got {}",
            cp.killed
        );
    }

    /// A `SwapRules` mutant may only pair rules that render under the same
    /// conditions. Swapping across a branch or a loop boundary moves an
    /// instruction in or out of a guarded region, which is not a reordering.
    #[test]
    fn test_swap_rules_only_within_one_guard_region() {
        let swaps = |t: &str| -> Vec<String> {
            generate_mutants(t)
                .into_iter()
                .filter(|m| m.operator == MutationOperator::SwapRules)
                .map(|m| m.id)
                .collect()
        };

        // Same region: swappable.
        let same = "// @rule RULE:A\nA body\n// @rule RULE:B\nB body";
        assert_eq!(swaps(same), vec!["SwapRules@L1-3"]);

        // if-arm vs else-arm: never co-render, so not a reordering.
        let arms = "{% if x %}\n// @rule RULE:A\nA body\n{% else %}\n// @rule RULE:B\nB body\n{% endif %}";
        assert!(swaps(arms).is_empty(), "{:?}", swaps(arms));

        // Outside vs inside an if: the swap would move B out of the guard.
        let nested = "// @rule RULE:A\nA body\n{% if x %}\n// @rule RULE:B\nB body\n{% endif %}";
        assert!(swaps(nested).is_empty(), "{:?}", swaps(nested));

        // Outside vs inside a loop body: same problem, and it would also change
        // how many times the instruction is emitted.
        let loops = "// @rule RULE:A\nA body\n{% for i in xs %}\n// @rule RULE:B\nB body\n{% endfor %}";
        assert!(swaps(loops).is_empty(), "{:?}", swaps(loops));

        // Two rules inside the same if body: still a legitimate reordering.
        let inside = "{% if x %}\n// @rule RULE:A\nA body\n// @rule RULE:B\nB body\n{% endif %}";
        assert_eq!(swaps(inside), vec!["SwapRules@L2-4"]);

        // Two separate if blocks with the same condition are still two regions:
        // a context can render one and not the other.
        let twin = "{% if x %}\n// @rule RULE:A\nA body\n{% endif %}\n{% if x %}\n// @rule RULE:B\nB body\n{% endif %}";
        assert!(swaps(twin).is_empty(), "{:?}", swaps(twin));
    }

    /// A swap that survives the guard check really does reorder the rendered text.
    #[test]
    fn test_swap_rules_reorders_rendered_text() {
        let t = "// @rule RULE:A\nA body\n// @rule RULE:B\nB body\n";
        let m = generate_mutants(t)
            .into_iter()
            .find(|m| m.operator == MutationOperator::SwapRules)
            .expect("a same-region swap");
        let render = |src: &str| {
            prompt_template::render(src, &json!({}))
                .unwrap()
                .output
                .replace('\n', "|")
        };
        assert_eq!(render(t), "A body|B body");
        assert_eq!(render(&m.source), "B body|A body");
    }

    /// A rule annotation with no body under it labels nothing, so dropping it is a
    /// metadata-only edit. Such sites are kept but tagged, so the two classes can
    /// be counted separately.
    #[test]
    fn test_bodiless_rule_drop_is_tagged_metadata_only() {
        // RULE:A is followed immediately by another annotation, RULE:B by a
        // control tag: neither labels a body.
        let t = "// @rule RULE:A\n// @rule RULE:B\n{% if x %}\ntext\n{% endif %}\n// @rule RULE:C\nC body";
        let drops: Vec<(u32, bool)> = generate_mutants(t)
            .into_iter()
            .filter(|m| m.operator == MutationOperator::DropRule)
            .map(|m| (m.line, m.metadata_only))
            .collect();
        assert_eq!(drops, vec![(0, true), (1, true), (5, false)]);

        // And the classification is reported per operator.
        let ir = build_ir(t, "file:///t", 1);
        let report = run_mutation_testing_default(&ir, t, &[json!({ "x": true })]).unwrap();
        let dr = report.by_operator.get("DropRule").expect("DropRule op");
        assert_eq!((dr.total, dr.metadata_only), (3, 2));
        // Annotation-only operators are metadata-only by construction.
        for op in ["ChangeCategory", "DropCondition", "ChangePriority"] {
            if let Some(s) = report.by_operator.get(op) {
                assert_eq!(s.total, s.metadata_only, "{op} should be metadata-only");
            }
        }
        // NegateCondition edits template control flow, which the model does see.
        let nc = report
            .by_operator
            .get("NegateCondition")
            .expect("NegateCondition op");
        assert_eq!(nc.metadata_only, 0);
    }

    /// Line removal must not rewrite the template's trailing newline. Dropping the
    /// final annotation of a newline-terminated file used to also swallow the
    /// terminator of the line before it, so the mutant rendered one newline short
    /// of the original and the output oracle scored a kill nobody made.
    #[test]
    fn test_line_removal_is_newline_consistent() {
        for t in [
            "a\nb\n",
            "a\nb",
            "a\n\n",
            "\n",
            "// @rule RULE:A\nbody\n",
            "",
        ] {
            let (lines, nl) = split_lines(t);
            assert_eq!(join(&lines, nl), t, "round-trip failed for {t:?}");
        }

        // The corpus case: a trailing `// @condition` after a blank line.
        let t = "{% if x %}\nyes\n{% else %}\nno\n{% endif %}\n\n// @condition CONDITION:C\n";
        let m = generate_mutants(t)
            .into_iter()
            .find(|m| m.operator == MutationOperator::DropCondition)
            .expect("a DropCondition mutant");
        assert!(m.source.ends_with('\n'), "{:?}", m.source);

        let ctx = json!({ "x": false });
        assert_eq!(
            prompt_template::render(t, &ctx).unwrap().output,
            prompt_template::render(&m.source, &ctx).unwrap().output,
            "dropping a stripped annotation must not change the rendered text"
        );

        use crate::diff::OutputOracle;
        let ir = build_ir(t, "file:///t", 1);
        let report = run_mutation_testing(&ir, t, &[ctx], &OutputOracle).unwrap();
        let dc = report
            .by_operator
            .get("DropCondition")
            .expect("DropCondition op");
        assert_eq!(dc.killed, 0, "an annotation-only drop is invisible to output");
    }

    #[test]
    fn test_mutation_score_improves_with_coverage() {
        let ir = build_ir(T, "file:///t", 1);

        // A single admin context cannot reach the else branch's rule, so some
        // mutants survive.
        let partial = run_mutation_testing_default(&ir, T, &[json!({"admin": true})]).unwrap();

        // Exercising both branches kills strictly more (or equal) mutants.
        let full = run_mutation_testing_default(
            &ir,
            T,
            &[json!({"admin": true}), json!({"admin": false})],
        )
        .unwrap();

        assert!(full.total_mutants > 0);
        assert!(full.killed >= partial.killed);
        assert!(full.mutation_score >= partial.mutation_score);
        assert_eq!(full.killed + full.survived, full.total_mutants);
    }
}

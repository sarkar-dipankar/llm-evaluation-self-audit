//! Deterministic, coverage-directed input-context generation.
//!
//! Given a `.prompt.rtpl` template, [`generate_contexts`] statically extracts the
//! variables it references and the conditions of its `{% if %}` branches, then emits
//! a small set of input contexts engineered to exercise **both outcomes of every
//! branch** while providing every referenced variable. This makes coverage and
//! mutation measurements meaningful without hand-authoring inputs per prompt.
//!
//! It is intentionally deterministic (no LLM): the same template always yields the
//! same contexts, so coverage results are reproducible.
//!
//! Loop collections are set to empty arrays (loops render zero iterations) so that
//! rendering never fails on unknown per-item fields; conditions inside loop bodies
//! are therefore not covered by the generated set (a documented limitation).

use serde_json::{json, Map, Value};

/// A parsed `{% if %}` condition with the assignments that make it true/false.
struct Cond {
    path: String,
    truthy: Value,
    falsy: Value,
    /// (ancestor condition index, required truth value) for enclosing branches.
    ancestors: Vec<(usize, bool)>,
}

enum Frame {
    If { cond_idx: usize, in_else: bool },
    For,
}

/// Generate input contexts that aim to cover both outcomes of every branch.
pub fn generate_contexts(template: &str) -> Vec<Value> {
    let conds = extract_conditions(template);
    let interp_paths = extract_interpolation_paths(template);
    let loop_colls = extract_loop_collections(template);

    // Base context: every interpolation var present, every condition falsy, every
    // loop collection empty. This alone exercises the false side of top-level branches.
    // A variable used as a loop collection must be an array. If it is also a
    // condition variable, its truthy/falsy assignments become non-empty/empty arrays
    // (truthy and iterable vs falsy and iterable) instead of booleans.
    let is_coll = |p: &str| loop_colls.iter().any(|c| c == p);
    let truthy_of = |c: &Cond| {
        if is_coll(&c.path) {
            json!(["item"])
        } else {
            c.truthy.clone()
        }
    };
    let falsy_of = |c: &Cond| {
        if is_coll(&c.path) {
            json!([])
        } else {
            c.falsy.clone()
        }
    };

    let mut base = Value::Object(Map::new());
    for p in &interp_paths {
        set_path(&mut base, p, json!("x"));
    }
    for c in &conds {
        set_path(&mut base, &c.path, falsy_of(c));
    }
    // Loop collections last so plain (non-condition) collections are empty arrays.
    for coll in &loop_colls {
        if !conds.iter().any(|c| &c.path == coll) {
            set_path(&mut base, coll, json!([]));
        }
    }

    let mut contexts = vec![base.clone()];

    // One context per condition that takes its true side (with ancestors satisfied).
    for c in &conds {
        let mut ctx = base.clone();
        for &(anc_idx, req_true) in &c.ancestors {
            let anc = &conds[anc_idx];
            set_path(
                &mut ctx,
                &anc.path,
                if req_true {
                    truthy_of(anc)
                } else {
                    falsy_of(anc)
                },
            );
        }
        set_path(&mut ctx, &c.path, truthy_of(c));
        contexts.push(ctx);
    }

    dedup(contexts)
}

fn dedup(contexts: Vec<Value>) -> Vec<Value> {
    let mut seen = Vec::new();
    let mut out = Vec::new();
    for c in contexts {
        let key = serde_json::to_string(&c).unwrap_or_default();
        if !seen.contains(&key) {
            seen.push(key);
            out.push(c);
        }
    }
    out
}

/// Extract conditions from `{% if %}` blocks, tracking the enclosing branch guards.
fn extract_conditions(template: &str) -> Vec<Cond> {
    let mut conds: Vec<Cond> = Vec::new();
    let mut stack: Vec<Frame> = Vec::new();

    for line in template.lines() {
        let t = line.trim();
        let inner = t
            .strip_prefix("{%")
            .and_then(|s| s.strip_suffix("%}"))
            .map(str::trim);
        let Some(inner) = inner else { continue };

        if let Some(cond) = inner.strip_prefix("if ") {
            if let Some((path, truthy, falsy)) = parse_condition(cond.trim()) {
                let ancestors = stack
                    .iter()
                    .filter_map(|f| match f {
                        Frame::If { cond_idx, in_else } => Some((*cond_idx, !*in_else)),
                        Frame::For => None,
                    })
                    .collect();
                let idx = conds.len();
                conds.push(Cond {
                    path,
                    truthy,
                    falsy,
                    ancestors,
                });
                stack.push(Frame::If {
                    cond_idx: idx,
                    in_else: false,
                });
            } else {
                // Unparseable condition still needs a stack frame for correct nesting.
                stack.push(Frame::If {
                    cond_idx: usize::MAX,
                    in_else: false,
                });
            }
        } else if inner == "else" {
            if let Some(Frame::If { in_else, .. }) = stack.last_mut() {
                *in_else = true;
            }
        } else if inner == "endif" {
            stack.pop();
        } else if inner.starts_with("for ") {
            stack.push(Frame::For);
        } else if inner == "endfor" {
            stack.pop();
        }
    }
    // Drop conditions whose ancestor chain references an unparseable (MAX) guard.
    conds.retain(|c| c.ancestors.iter().all(|&(i, _)| i != usize::MAX));
    conds
}

/// Parse a condition into (path, truthy-assignment, falsy-assignment).
fn parse_condition(cond: &str) -> Option<(String, Value, Value)> {
    let c = cond.trim();
    if let Some(rest) = c.strip_prefix("not ").or_else(|| c.strip_prefix('!')) {
        return Some((rest.trim().to_string(), json!(false), json!(true)));
    }
    if let Some((l, r)) = c.split_once("==") {
        let (path, lit) = pick_path_literal(l.trim(), r.trim());
        let v = literal_value(lit);
        let other = other_value(&v);
        return Some((path, v, other));
    }
    if let Some((l, r)) = c.split_once("!=") {
        let (path, lit) = pick_path_literal(l.trim(), r.trim());
        let v = literal_value(lit);
        let other = other_value(&v);
        return Some((path, other, v));
    }
    if is_identifier_path(c) {
        return Some((c.to_string(), json!(true), json!(false)));
    }
    None
}

/// Return (path, literal) given the two sides of a comparison.
fn pick_path_literal<'a>(l: &'a str, r: &'a str) -> (String, &'a str) {
    if is_literal(l) {
        (r.to_string(), l)
    } else {
        (l.to_string(), r)
    }
}

fn is_literal(s: &str) -> bool {
    s.starts_with('"')
        || s.starts_with('\'')
        || s == "true"
        || s == "false"
        || s.parse::<f64>().is_ok()
}

fn is_identifier_path(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
}

fn literal_value(lit: &str) -> Value {
    let l = lit.trim();
    if (l.starts_with('"') && l.ends_with('"')) || (l.starts_with('\'') && l.ends_with('\'')) {
        return json!(l[1..l.len() - 1].to_string());
    }
    if l == "true" {
        return json!(true);
    }
    if l == "false" {
        return json!(false);
    }
    if let Ok(n) = l.parse::<i64>() {
        return json!(n);
    }
    if let Ok(f) = l.parse::<f64>() {
        return json!(f);
    }
    json!(l.to_string())
}

/// A distinct value of the same shape (so the condition flips).
fn other_value(v: &Value) -> Value {
    match v {
        Value::Bool(b) => json!(!b),
        Value::Number(n) => json!(n.as_i64().unwrap_or(0) + 1),
        Value::String(s) => json!(format!("__not_{s}__")),
        _ => json!("__other__"),
    }
}

fn extract_interpolation_paths(template: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let bytes = template.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            if let Some(end) = template[i + 2..].find("}}") {
                let inner = template[i + 2..i + 2 + end].trim();
                // Take the leading dot-path token only.
                let token: String = inner
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
                    .collect();
                if is_identifier_path(&token) && !paths.contains(&token) {
                    paths.push(token);
                }
                i += 2 + end + 2;
                continue;
            }
        }
        i += 1;
    }
    paths
}

fn extract_loop_collections(template: &str) -> Vec<String> {
    let mut colls = Vec::new();
    for line in template.lines() {
        let t = line.trim();
        if let Some(inner) = t.strip_prefix("{%").and_then(|s| s.strip_suffix("%}")) {
            if let Some(for_expr) = inner.trim().strip_prefix("for ") {
                if let Some((_, coll)) = for_expr.split_once(" in ") {
                    let coll = coll.trim().to_string();
                    if is_identifier_path(&coll) && !colls.contains(&coll) {
                        colls.push(coll);
                    }
                }
            }
        }
    }
    colls
}

/// Set a dot-path within a JSON object value, creating intermediate objects.
fn set_path(root: &mut Value, path: &str, val: Value) {
    let parts: Vec<&str> = path.split('.').collect();
    if parts.is_empty() {
        return;
    }
    let mut cur = root;
    for p in &parts[..parts.len() - 1] {
        if !cur.is_object() {
            *cur = json!({});
        }
        cur = cur
            .as_object_mut()
            .unwrap()
            .entry((*p).to_string())
            .or_insert_with(|| json!({}));
    }
    if !cur.is_object() {
        *cur = json!({});
    }
    cur.as_object_mut()
        .unwrap()
        .insert(parts[parts.len() - 1].to_string(), val);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render;

    fn branch_outcomes(template: &str) -> (bool, bool) {
        // Returns (saw_true, saw_false) for the FIRST branch across generated contexts.
        let ctxs = generate_contexts(template);
        let mut t = false;
        let mut f = false;
        for c in &ctxs {
            if let Ok(r) = render(template, c) {
                if let Some(b) = r.branches_taken.first() {
                    if b.taken {
                        t = true;
                    } else {
                        f = true;
                    }
                }
            }
        }
        (t, f)
    }

    #[test]
    fn test_covers_both_sides_string_eq() {
        let t =
            "{% if user.role == \"admin\" %}\nA\n{% else %}\nB\n{% endif %}\n{{ user.message }}";
        let (saw_t, saw_f) = branch_outcomes(t);
        assert!(
            saw_t && saw_f,
            "expected both outcomes, got t={saw_t} f={saw_f}"
        );
    }

    #[test]
    fn test_covers_truthy_and_negation() {
        assert_eq!(
            branch_outcomes("{% if flag %}\nX\n{% endif %}"),
            (true, true)
        );
        assert_eq!(
            branch_outcomes("{% if not flag %}\nX\n{% endif %}"),
            (true, true)
        );
    }

    #[test]
    fn test_numeric_condition() {
        let (t, f) = branch_outcomes("{% if value == 42 %}\nX\n{% endif %}");
        assert!(t && f);
    }

    #[test]
    fn test_render_never_fails_with_generated_contexts() {
        // Includes a loop and interpolation: generated contexts must render cleanly.
        let t = "{% if a.b == \"x\" %}\n{{ a.c }}\n{% endif %}\n{% for it in items %}\n- loop\n{% endfor %}";
        for c in generate_contexts(t) {
            assert!(render(t, &c).is_ok(), "render failed for ctx {c}");
        }
    }

    #[test]
    fn test_deterministic() {
        let t = "{% if user.tier == \"vip\" %}\nV\n{% endif %}";
        let a = serde_json::to_string(&generate_contexts(t)).unwrap();
        let b = serde_json::to_string(&generate_contexts(t)).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn test_nested_condition_true_side_reachable() {
        // Inner condition only reachable when outer is true.
        let t = "{% if region == \"eu\" %}\n{% if minor %}\nM\n{% endif %}\n{% endif %}";
        let ctxs = generate_contexts(t);
        // Some context must take BOTH the outer-if and inner-if true.
        let reached = ctxs.iter().any(|c| {
            render(t, c)
                .map(|r| r.branches_taken.iter().filter(|b| b.taken).count() == 2)
                .unwrap_or(false)
        });
        assert!(reached, "no context reached the nested true/true path");
    }
}

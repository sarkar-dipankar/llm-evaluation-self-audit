//! IR stability and regression tests.
//!
//! These tests ensure that:
//! - IR generation is deterministic
//! - IR structure matches expected schema
//! - Node IDs are stable across runs
//! - Range information is accurate

#[cfg(test)]
mod tests {
    use crate::fixtures::*;
    use pretty_assertions::assert_eq;
    use prompt_ir::{IRKind, IRNode, IRNodeMeta, PromptIR, TextRange};
    use prompt_template::AnnotationKind;

    /// Verify that annotations are correctly parsed into IR nodes.
    #[test]
    fn test_annotation_parsing() {
        // Parse annotations from the annotated prompt
        let annotations = prompt_template::parse_annotations(ANNOTATED_PROMPT);

        assert_eq!(annotations.len(), 4);

        // Verify rule annotations
        let rules: Vec<_> = annotations
            .iter()
            .filter(|a| a.kind == AnnotationKind::Rule)
            .collect();
        assert_eq!(rules.len(), 2);
        assert!(rules.iter().any(|r| r.id == "RULE:NO_HARMFUL_CONTENT"));
        assert!(rules.iter().any(|r| r.id == "RULE:STAY_ON_TOPIC"));

        // Verify condition annotation
        let conditions: Vec<_> = annotations
            .iter()
            .filter(|a| a.kind == AnnotationKind::Condition)
            .collect();
        assert_eq!(conditions.len(), 1);
        assert_eq!(conditions[0].id, "CONDITION:CODE_MODE");

        // Verify breakpoint annotation
        let breakpoints: Vec<_> = annotations
            .iter()
            .filter(|a| a.kind == AnnotationKind::Breakpoint)
            .collect();
        assert_eq!(breakpoints.len(), 1);
        assert_eq!(breakpoints[0].id, "BP:USER_INPUT");
    }

    /// Verify that conditional blocks create appropriate structure.
    #[test]
    fn test_conditional_block_detection() {
        let result =
            prompt_template::render(CONDITIONAL_PROMPT, &conditional_context_vip()).unwrap();

        // Should have taken the VIP branch
        assert_eq!(result.branches_taken.len(), 1);
        assert!(result.branches_taken[0].taken);
        assert_eq!(result.branches_taken[0].condition, "is_vip");
    }

    /// Verify that variables are correctly tracked.
    #[test]
    fn test_variable_tracking() {
        let result = prompt_template::render(SIMPLE_PROMPT, &simple_context()).unwrap();

        assert!(result.variables_used.contains(&"user_input".to_string()));
        assert_eq!(result.variable_accesses.len(), 1);
        assert_eq!(result.variable_accesses[0].name, "user_input");
        assert_eq!(result.variable_accesses[0].value_type, "string");
    }

    /// Verify IR node structure matches expected format.
    #[test]
    fn test_ir_node_structure() {
        let node = IRNode {
            id: "RULE:TEST".to_string(),
            kind: IRKind::Rule,
            label: Some("Test rule".to_string()),
            range: TextRange {
                start_line: 0,
                start_col: 0,
                end_line: 0,
                end_col: 20,
            },
            meta: IRNodeMeta::default(),
        };

        // Verify JSON serialization is stable
        let json = serde_json::to_string(&node).unwrap();
        let parsed: IRNode = serde_json::from_str(&json).unwrap();

        assert_eq!(node.id, parsed.id);
        assert_eq!(node.kind, parsed.kind);
        assert_eq!(node.label, parsed.label);
        assert_eq!(node.range.start_line, parsed.range.start_line);
    }

    /// Verify PromptIR helper methods work correctly.
    #[test]
    fn test_prompt_ir_helpers() {
        let ir = PromptIR {
            uri: "test.prompt.rtpl".to_string(),
            version: 1,
            nodes: vec![
                IRNode {
                    id: "RULE:A".to_string(),
                    kind: IRKind::Rule,
                    label: Some("Rule A".to_string()),
                    range: TextRange::default(),
                    meta: IRNodeMeta::default(),
                },
                IRNode {
                    id: "CONDITION:B".to_string(),
                    kind: IRKind::Condition,
                    label: Some("Condition B".to_string()),
                    range: TextRange::default(),
                    meta: IRNodeMeta::default(),
                },
                IRNode {
                    id: "BP:C".to_string(),
                    kind: IRKind::Breakpoint,
                    label: Some("Breakpoint C".to_string()),
                    range: TextRange::default(),
                    meta: IRNodeMeta::default(),
                },
            ],
            meta: None,
        };

        // Test rules() helper
        let rules = ir.rules();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].id, "RULE:A");

        // Test conditions() helper
        let conditions = ir.conditions();
        assert_eq!(conditions.len(), 1);
        assert_eq!(conditions[0].id, "CONDITION:B");

        // Test breakpoints() helper
        let breakpoints = ir.breakpoints();
        assert_eq!(breakpoints.len(), 1);
        assert_eq!(breakpoints[0].id, "BP:C");

        // Test find_node()
        assert!(ir.find_node("RULE:A").is_some());
        assert!(ir.find_node("NONEXISTENT").is_none());
    }

    /// Verify range information is preserved through serialization.
    #[test]
    fn test_range_serialization() {
        let range = TextRange {
            start_line: 10,
            start_col: 5,
            end_line: 15,
            end_col: 20,
        };

        let json = serde_json::to_string(&range).unwrap();
        let parsed: TextRange = serde_json::from_str(&json).unwrap();

        assert_eq!(range.start_line, parsed.start_line);
        assert_eq!(range.start_col, parsed.start_col);
        assert_eq!(range.end_line, parsed.end_line);
        assert_eq!(range.end_col, parsed.end_col);
    }
}

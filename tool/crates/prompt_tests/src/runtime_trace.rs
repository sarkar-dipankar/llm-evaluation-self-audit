//! Runtime trace and execution tests.
//!
//! These tests verify:
//! - Execution traces are correctly generated
//! - Breakpoints are hit at expected locations
//! - Variables are correctly captured in traces
//! - Error conditions produce appropriate diagnostics

#[cfg(test)]
mod tests {
    use crate::fixtures::*;
    use pretty_assertions::assert_eq;
    use prompt_ir::{TraceStep, TraceStepKind};
    use prompt_template::AnnotationKind;

    /// Verify trace step creation and serialization.
    #[test]
    fn test_trace_step_creation() {
        let step =
            TraceStep::new(0, TraceStepKind::Plan).with_explanation("Starting template rendering");

        assert_eq!(step.index, 0);
        assert_eq!(step.kind, TraceStepKind::Plan);
        assert_eq!(
            step.explanation,
            Some("Starting template rendering".to_string())
        );
    }

    /// Verify trace step kinds are serialized correctly.
    #[test]
    fn test_trace_step_kinds() {
        let kinds = vec![
            TraceStepKind::Plan,
            TraceStepKind::Execution,
            TraceStepKind::Breakpoint,
            TraceStepKind::Summary,
        ];

        for kind in kinds {
            let json = serde_json::to_string(&kind).unwrap();
            let parsed: TraceStepKind = serde_json::from_str(&json).unwrap();
            assert_eq!(kind, parsed);
        }
    }

    /// Verify template rendering produces correct trace.
    #[test]
    fn test_template_trace_generation() {
        let result = prompt_template::render(SIMPLE_PROMPT, &simple_context()).unwrap();

        // Should have variable accesses
        assert!(!result.variable_accesses.is_empty());

        // First variable access should be user_input
        assert_eq!(result.variable_accesses[0].name, "user_input");
    }

    /// Verify conditional branches generate appropriate trace info.
    #[test]
    fn test_conditional_trace() {
        let result_vip =
            prompt_template::render(CONDITIONAL_PROMPT, &conditional_context_vip()).unwrap();
        let result_regular =
            prompt_template::render(CONDITIONAL_PROMPT, &conditional_context_regular()).unwrap();

        // VIP path should be taken
        assert!(result_vip.branches_taken[0].taken);

        // Non-VIP path should not take the if branch
        assert!(!result_regular.branches_taken[0].taken);
    }

    /// Verify nested conditionals produce correct trace.
    #[test]
    fn test_nested_conditional_trace() {
        let result =
            prompt_template::render(NESTED_CONDITIONAL_PROMPT, &nested_admin_delete_context())
                .unwrap();

        // Should have 2 branches evaluated (outer and inner)
        assert_eq!(result.branches_taken.len(), 2);

        // Both should be taken for admin + delete
        assert!(result.branches_taken.iter().all(|b| b.taken));
    }

    /// Verify loops produce iteration trace info.
    #[test]
    fn test_loop_trace() {
        let result = prompt_template::render(LOOP_PROMPT, &loop_context()).unwrap();

        // Should have loop iteration info
        assert!(!result.loop_iterations.is_empty());

        // Should have 3 iterations (3 items)
        assert_eq!(result.loop_iterations[0].iteration_count, 3);
    }

    /// Verify template errors are properly captured.
    #[test]
    fn test_error_trace() {
        // Missing required variable
        let result = prompt_template::render(SIMPLE_PROMPT, &serde_json::json!({}));

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("user_input"));
    }

    /// Verify breakpoint location detection.
    #[test]
    fn test_breakpoint_detection() {
        let annotations = prompt_template::parse_annotations(ANNOTATED_PROMPT);

        let breakpoints: Vec<_> = annotations
            .iter()
            .filter(|a| a.kind == AnnotationKind::Breakpoint)
            .collect();

        assert_eq!(breakpoints.len(), 1);
        assert_eq!(breakpoints[0].id, "BP:USER_INPUT");

        // Verify range is captured
        assert!(breakpoints[0].range.start_line > 0 || breakpoints[0].range.start_col > 0);
    }

    /// Verify variable types are correctly identified.
    #[test]
    fn test_variable_type_detection() {
        let context = serde_json::json!({
            "string_var": "hello",
            "number_var": 42,
            "bool_var": true,
            "array_var": [1, 2, 3],
            "object_var": {"key": "value"}
        });

        let template = r#"
{{ string_var }}
{{ number_var }}
{{ bool_var }}
{{ array_var }}
{{ object_var }}
"#;

        let result = prompt_template::render(template, &context).unwrap();

        let types: std::collections::HashMap<_, _> = result
            .variable_accesses
            .iter()
            .map(|a| (a.name.as_str(), a.value_type.as_str()))
            .collect();

        assert_eq!(types.get("string_var"), Some(&"string"));
        assert_eq!(types.get("number_var"), Some(&"number"));
        assert_eq!(types.get("bool_var"), Some(&"boolean"));
        assert_eq!(types.get("array_var"), Some(&"array"));
        assert_eq!(types.get("object_var"), Some(&"object"));
    }

    /// Verify trace step builder methods work correctly.
    #[test]
    fn test_trace_step_builder() {
        let step = TraceStep::new(0, TraceStepKind::Execution)
            .with_explanation("Applying rule")
            .with_rules_used(vec!["RULE:A".to_string()])
            .with_partial_answer("Hello")
            .with_breakpoint_hit("BP:TEST");

        assert_eq!(step.kind, TraceStepKind::Execution);
        assert_eq!(step.explanation, Some("Applying rule".to_string()));
        assert_eq!(step.rules_used, vec!["RULE:A"]);
        assert_eq!(step.partial_answer, Some("Hello".to_string()));
        assert_eq!(step.breakpoints_hit, vec!["BP:TEST"]);
    }
}

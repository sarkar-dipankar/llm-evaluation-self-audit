//! Template rendering regression tests.
//!
//! These tests ensure:
//! - Template output is stable across versions
//! - Edge cases are handled correctly
//! - Error messages are helpful and consistent

#[cfg(test)]
mod tests {
    use crate::fixtures::*;
    use pretty_assertions::assert_eq;

    /// Verify simple variable interpolation.
    #[test]
    fn test_simple_interpolation() {
        let result = prompt_template::render(SIMPLE_PROMPT, &simple_context()).unwrap();

        assert!(result.output.contains("Hello, how are you?"));
        assert!(result.output.contains("You are a helpful assistant."));
    }

    /// Verify conditional rendering with VIP context.
    #[test]
    fn test_conditional_vip() {
        let result =
            prompt_template::render(CONDITIONAL_PROMPT, &conditional_context_vip()).unwrap();

        assert!(result.output.contains("Prioritize this customer's request"));
        assert!(result.output.contains("VIP member"));
        assert!(!result.output.contains("standard priority"));
    }

    /// Verify conditional rendering with regular context.
    #[test]
    fn test_conditional_regular() {
        let result =
            prompt_template::render(CONDITIONAL_PROMPT, &conditional_context_regular()).unwrap();

        assert!(result.output.contains("standard priority"));
        assert!(!result.output.contains("VIP member"));
    }

    /// Verify loop rendering.
    #[test]
    fn test_loop_rendering() {
        let result = prompt_template::render(LOOP_PROMPT, &loop_context()).unwrap();

        assert!(result.output.contains("Item A: 100"));
        assert!(result.output.contains("Item B: 200"));
        assert!(result.output.contains("Item C: 300"));
        assert!(result.output.contains("Total: 600"));
    }

    /// Verify nested conditionals.
    #[test]
    fn test_nested_conditionals_admin_delete() {
        let result =
            prompt_template::render(NESTED_CONDITIONAL_PROMPT, &nested_admin_delete_context())
                .unwrap();

        assert!(result
            .output
            .contains("Confirm deletion with admin privileges"));
    }

    /// Verify nested conditionals for user delete.
    #[test]
    fn test_nested_conditionals_user_delete() {
        let result =
            prompt_template::render(NESTED_CONDITIONAL_PROMPT, &nested_user_delete_context())
                .unwrap();

        assert!(result.output.contains("Deletion requires admin privileges"));
    }

    /// Verify annotations are stripped from the rendered prompt (they are toolchain
    /// metadata) but still parsed into the structure.
    #[test]
    fn test_annotations_stripped_from_output_but_parsed() {
        let result = prompt_template::render(ANNOTATED_PROMPT, &annotated_context()).unwrap();

        // Annotation comment lines must NOT leak into the prompt the model sees.
        assert!(!result.output.contains("// @rule RULE:NO_HARMFUL_CONTENT"));
        assert!(!result.output.contains("// @breakpoint BP:USER_INPUT"));

        // But they are still recoverable from the source as structured annotations.
        let annotations = prompt_template::parse_annotations(ANNOTATED_PROMPT);
        assert!(annotations
            .iter()
            .any(|a| a.id == "RULE:NO_HARMFUL_CONTENT"));
        assert!(annotations.iter().any(|a| a.id == "BP:USER_INPUT"));
    }

    /// Verify whitespace handling in output.
    #[test]
    fn test_whitespace_handling() {
        let template = "Line 1\n{{ var }}\nLine 3";
        let context = serde_json::json!({"var": "Line 2"});

        let result = prompt_template::render(template, &context).unwrap();

        assert_eq!(result.output, "Line 1\nLine 2\nLine 3");
    }

    /// Verify empty string handling.
    #[test]
    fn test_empty_string_variable() {
        let template = "Before {{ var }} After";
        let context = serde_json::json!({"var": ""});

        let result = prompt_template::render(template, &context).unwrap();

        assert_eq!(result.output, "Before  After");
    }

    /// Verify number formatting.
    #[test]
    fn test_number_formatting() {
        let template = "Value: {{ num }}";
        let context = serde_json::json!({"num": 42.5});

        let result = prompt_template::render(template, &context).unwrap();

        assert_eq!(result.output, "Value: 42.5");
    }

    /// Verify boolean rendering.
    #[test]
    fn test_boolean_rendering() {
        let template = "Flag: {{ flag }}";

        let true_ctx = serde_json::json!({"flag": true});
        let false_ctx = serde_json::json!({"flag": false});

        let true_result = prompt_template::render(template, &true_ctx).unwrap();
        let false_result = prompt_template::render(template, &false_ctx).unwrap();

        assert_eq!(true_result.output, "Flag: true");
        assert_eq!(false_result.output, "Flag: false");
    }

    /// Verify null rendering.
    #[test]
    fn test_null_rendering() {
        let template = "Value: {{ val }}";
        let context = serde_json::json!({"val": null});

        let result = prompt_template::render(template, &context).unwrap();

        assert_eq!(result.output, "Value: null");
    }

    /// Verify array access in loops.
    #[test]
    fn test_loop_array_access() {
        let template = r#"{% for n in numbers %}
{{ n }}
{% endfor %}"#;
        let context = serde_json::json!({"numbers": [1, 2, 3, 4, 5]});

        let result = prompt_template::render(template, &context).unwrap();

        assert!(result.output.contains("1"));
        assert!(result.output.contains("5"));
    }

    /// Verify nested object access.
    #[test]
    fn test_nested_object_access() {
        let template = "{{ user.profile.name }}";
        let context = serde_json::json!({
            "user": {
                "profile": {
                    "name": "Alice"
                }
            }
        });

        let result = prompt_template::render(template, &context).unwrap();

        assert_eq!(result.output, "Alice");
    }

    /// Verify error on missing variable.
    #[test]
    fn test_missing_variable_error() {
        let template = "Hello {{ name }}";
        let context = serde_json::json!({});

        let result = prompt_template::render(template, &context);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("name"));
    }

    /// Verify error on missing nested property.
    #[test]
    fn test_missing_nested_property_error() {
        let template = "{{ user.nonexistent }}";
        let context = serde_json::json!({"user": {}});

        let result = prompt_template::render(template, &context);

        assert!(result.is_err());
    }

    /// Verify if-else-if chains.
    #[test]
    fn test_if_else_if_chain() {
        let template = r#"{% if level == "high" %}
HIGH
{% else %}
{% if level == "medium" %}
MEDIUM
{% else %}
LOW
{% endif %}
{% endif %}"#;

        let high =
            prompt_template::render(template, &serde_json::json!({"level": "high"})).unwrap();
        let medium =
            prompt_template::render(template, &serde_json::json!({"level": "medium"})).unwrap();
        let low = prompt_template::render(template, &serde_json::json!({"level": "low"})).unwrap();

        assert!(high.output.contains("HIGH"));
        assert!(medium.output.contains("MEDIUM"));
        assert!(low.output.contains("LOW"));
    }

    /// Verify comparison operators.
    #[test]
    fn test_comparison_operators() {
        let eq_template = r#"{% if a == 1 %}
equal
{% else %}
not equal
{% endif %}"#;

        let equal = prompt_template::render(eq_template, &serde_json::json!({"a": 1})).unwrap();
        let not_equal = prompt_template::render(eq_template, &serde_json::json!({"a": 2})).unwrap();

        assert!(equal.output.contains("equal"));
        assert!(not_equal.output.contains("not equal"));
    }

    /// Verify negation operator.
    #[test]
    fn test_negation_operator() {
        let template = r#"{% if !flag %}
NOT SET
{% else %}
SET
{% endif %}"#;

        let not_set =
            prompt_template::render(template, &serde_json::json!({"flag": false})).unwrap();
        let set = prompt_template::render(template, &serde_json::json!({"flag": true})).unwrap();

        assert!(not_set.output.contains("NOT SET"));
        assert!(set.output.contains("SET"));
    }
}

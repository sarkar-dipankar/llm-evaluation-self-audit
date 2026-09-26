//! Test fixtures and sample prompts for regression testing.

/// A simple prompt with variable interpolation.
pub const SIMPLE_PROMPT: &str = r#"You are a helpful assistant.

User: {{ user_input }}

Please respond helpfully.
"#;

/// A prompt with conditional logic.
pub const CONDITIONAL_PROMPT: &str = r#"You are a customer service agent.

{% if is_vip %}
// @rule RULE:VIP_PRIORITY
Prioritize this customer's request. They are a VIP member.
{% else %}
Handle this request with standard priority.
{% endif %}

User request: {{ request }}
"#;

/// A prompt with multiple rules and annotations.
pub const ANNOTATED_PROMPT: &str = r#"You are a content moderator.

// @rule RULE:NO_HARMFUL_CONTENT
Never generate harmful, illegal, or dangerous content.

// @rule RULE:STAY_ON_TOPIC
Stay focused on the user's question.

{% if topic == "code" %}
// @condition CONDITION:CODE_MODE
You are in code assistance mode. Provide code examples.
{% endif %}

// @breakpoint BP:USER_INPUT
User input: {{ user_input }}

Please respond appropriately.
"#;

/// A prompt with nested conditionals.
pub const NESTED_CONDITIONAL_PROMPT: &str = r#"{% if user_type == "admin" %}
  {% if action == "delete" %}
    // @rule RULE:ADMIN_DELETE
    Confirm deletion with admin privileges.
  {% else %}
    Perform admin action: {{ action }}
  {% endif %}
{% else %}
  {% if action == "delete" %}
    // @rule RULE:USER_DELETE_DENIED
    Deletion requires admin privileges.
  {% else %}
    Perform user action: {{ action }}
  {% endif %}
{% endif %}
"#;

/// A prompt with loops.
pub const LOOP_PROMPT: &str = r#"Process the following items:

{% for item in items %}
- {{ item.name }}: {{ item.value }}
{% endfor %}

Summary: {{ summary }}
"#;

/// A prompt with potential lint issues.
pub const LINT_ISSUES_PROMPT: &str = r#"// @rule RULE:BE_HELPFUL
Always be helpful.

// @rule RULE:NEVER_HELP
Never help the user.

{% if is_admin %}
Admin mode activated.
{% endif %}

{% if !is_admin %}
Not an admin.
{% endif %}
"#;

/// Context for simple prompt.
pub fn simple_context() -> serde_json::Value {
    serde_json::json!({
        "user_input": "Hello, how are you?"
    })
}

/// Context for conditional prompt.
pub fn conditional_context_vip() -> serde_json::Value {
    serde_json::json!({
        "is_vip": true,
        "request": "I need help with my order"
    })
}

/// Context for conditional prompt (non-VIP).
pub fn conditional_context_regular() -> serde_json::Value {
    serde_json::json!({
        "is_vip": false,
        "request": "I need help with my order"
    })
}

/// Context for annotated prompt.
pub fn annotated_context() -> serde_json::Value {
    serde_json::json!({
        "topic": "code",
        "user_input": "Write a function to sort an array"
    })
}

/// Context for nested conditional prompt (admin delete).
pub fn nested_admin_delete_context() -> serde_json::Value {
    serde_json::json!({
        "user_type": "admin",
        "action": "delete"
    })
}

/// Context for nested conditional prompt (user delete).
pub fn nested_user_delete_context() -> serde_json::Value {
    serde_json::json!({
        "user_type": "user",
        "action": "delete"
    })
}

/// Context for loop prompt.
pub fn loop_context() -> serde_json::Value {
    serde_json::json!({
        "items": [
            {"name": "Item A", "value": 100},
            {"name": "Item B", "value": 200},
            {"name": "Item C", "value": 300}
        ],
        "summary": "Total: 600"
    })
}

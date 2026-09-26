//! Analysis prompts for IR generation, linting, and test generation.

/// Generate the IR generation prompt.
pub fn ir_generation_prompt(content: &str) -> String {
    format!(
        r#"You are a prompt analyzer. Parse the following prompt template and extract structured information.

INPUT FORMAT:
- Prompt templates use `{{{{ variable }}}}` for interpolation
- Logic blocks use `{{% if condition %}} ... {{% endif %}}`
- Annotations use `// @rule RULE:NAME`, `// @condition CONDITION:NAME`, `// @breakpoint BREAKPOINT:NAME`

OUTPUT FORMAT (JSON):
{{
  "nodes": [
    {{
      "id": "RULE:NAME or auto-generated",
      "kind": "Rule | Condition | Breakpoint | Section | Directive",
      "label": "human-readable description",
      "range": {{ "start_line": N, "start_col": N, "end_line": N, "end_col": N }},
      "meta": {{
        "category": "style | safety | format | null",
        "triggers": ["topic:X"],
        "condition_ref": "CONDITION:NAME or null",
        "priority": N or null,
        "inferred": true | false
      }}
    }}
  ],
  "meta": {{
    "parser_version": "1.0",
    "confidence": 0.0-1.0,
    "engine": "llm",
    "warnings": ["any parsing warnings"]
  }}
}}

RULES:
1. Explicit annotations (// @rule, etc.) should have inferred=false
2. Infer implicit rules from instruction patterns (e.g., "Always X" implies a rule)
3. Mark inferred nodes with inferred=true
4. Generate deterministic IDs: RULE:SNAKE_CASE_DESCRIPTION
5. Capture accurate line/column ranges (0-indexed)
6. Set confidence based on annotation clarity (1.0 for explicit, lower for inferred)

---
PROMPT TO ANALYZE:
{content}
---

Respond with only the JSON output, no explanation."#
    )
}

/// Generate the lint prompt.
pub fn lint_prompt(content: &str, ir_json: &str) -> String {
    format!(
        r#"You are a prompt linter. Analyze the following prompt template and IR for potential issues.

OUTPUT FORMAT (JSON):
{{
  "diagnostics": [
    {{
      "message": "Description of the issue",
      "range": {{ "start_line": N, "start_col": N, "end_line": N, "end_col": N }},
      "severity": "Error | Warning | Info",
      "code": "prompt/issue-code"
    }}
  ]
}}

ISSUE TYPES TO DETECT:

1. CONFLICTS (prompt/conflict-*)
   - Contradictory instructions
   - Conflicting rules with same trigger conditions

2. UNREACHABLE (prompt/unreachable-*)
   - Conditions that can never be true
   - Rules shadowed by higher-priority rules

3. MALFORMED (prompt/malformed-*)
   - Invalid annotation syntax
   - Unmatched template blocks

4. STYLE (prompt/style-*)
   - Vague instructions
   - Missing edge case handling

5. SAFETY (prompt/safety-*)
   - Missing guardrails
   - Potential prompt injection vulnerabilities

---
PROMPT TEMPLATE:
{content}

IR:
{ir_json}
---

Respond with only the JSON output, no explanation."#
    )
}

/// Generate the breakpoint suggestion prompt.
pub fn suggest_breakpoints_prompt(content: &str, ir_json: &str) -> String {
    format!(
        r#"You are a debugging assistant. Analyze the following prompt and IR to suggest strategic breakpoint locations.

A good breakpoint location is where:
1. Complex decision logic occurs
2. Edge cases are handled
3. Safety checks are performed
4. Output format changes
5. User input is first processed

OUTPUT FORMAT (JSON):
{{
  "suggestions": [
    {{
      "node_id": "RULE:NAME or CONDITION:NAME",
      "reason": "Why this is a good breakpoint location",
      "confidence": 0.0-1.0
    }}
  ]
}}

---
PROMPT TEMPLATE:
{content}

IR:
{ir_json}
---

Respond with only the JSON output, no explanation."#
    )
}

/// Generate the test generation prompt.
pub fn generate_tests_prompt(content: &str, node_json: &str, ir_json: &str) -> String {
    format!(
        r#"You are a test generator. Create test cases for the specified rule/condition in this prompt.

TEST TYPES:
1. Positive - Input that should trigger the rule
2. Negative - Input that should NOT trigger the rule
3. Edge - Boundary cases that test rule limits

OUTPUT FORMAT (JSON):
{{
  "tests": [
    {{
      "id": "test_unique_id",
      "target_node_id": "RULE:NAME",
      "kind": "Positive | Negative | Edge",
      "input": "The user input text to test with",
      "expected_behavior": "Description of expected LLM behavior",
      "comment": "Why this test case matters"
    }}
  ]
}}

Generate 2-3 tests per type (6-9 total).

---
PROMPT TEMPLATE:
{content}

TARGET NODE:
{node_json}

FULL IR:
{ir_json}
---

Respond with only the JSON output, no explanation."#
    )
}

/// Generate IR from a Claude Code skill file.
pub fn skill_ir_generation_prompt(content: &str, frontmatter_json: &str) -> String {
    format!(
        r#"You are a skill analyzer. Parse the following Claude Code SKILL.md file and extract structured information.

INPUT FORMAT:
- SKILL.md files have YAML frontmatter between --- delimiters
- Body contains Markdown instructions for Claude Code
- `$ARGUMENTS` is substituted with user arguments
- `${{{{CLAUDE_SESSION_ID}}}}` is the current session identifier
- `!command` executes shell commands dynamically before Claude sees the content
- Files in scripts/, references/, resources/ are skill resources

OUTPUT FORMAT (JSON):
{{
  "nodes": [
    {{
      "id": "INSTRUCTION:SNAKE_CASE_ID or CONSTRAINT:SNAKE_CASE_ID",
      "kind": "SkillInstruction | SkillConstraint",
      "label": "human-readable description",
      "range": {{ "start_line": N, "start_col": N, "end_line": N, "end_col": N }},
      "meta": {{
        "category": "behavior | safety | output | context | access | null",
        "priority": N or null,
        "inferred": true | false,
        "source": "frontmatter | body"
      }}
    }}
  ],
  "meta": {{
    "parser_version": "1.0",
    "confidence": 0.0-1.0,
    "engine": "llm",
    "warnings": ["any parsing warnings"]
  }}
}}

PARSING RULES:
1. Extract behavioral instructions from the body (imperatives like "Always...", "Never...", "When...", "Focus on...")
2. Convert frontmatter constraints to SkillConstraint nodes:
   - `allowed-tools` becomes a CONSTRAINT:TOOL_RESTRICTION
   - `disable-model-invocation: true` becomes CONSTRAINT:USER_ONLY
   - `user-invocable: false` becomes CONSTRAINT:MODEL_ONLY
3. Infer implicit constraints from instructions
4. Mark nodes with source="frontmatter" or source="body"
5. Generate deterministic IDs: INSTRUCTION:SNAKE_CASE or CONSTRAINT:SNAKE_CASE
6. Capture accurate line/column ranges (0-indexed, relative to full file)
7. Set confidence based on clarity (1.0 for explicit constraints, lower for inferred)

---
SKILL FRONTMATTER (pre-parsed):
{frontmatter_json}

SKILL CONTENT:
{content}
---

Respond with only the JSON output, no explanation."#
    )
}

/// Lint a Claude Code skill file.
pub fn skill_lint_prompt(content: &str, ir_json: &str, frontmatter_json: &str) -> String {
    format!(
        r#"You are a skill linter. Analyze the following SKILL.md file and IR for potential issues.

OUTPUT FORMAT (JSON):
{{
  "diagnostics": [
    {{
      "message": "Description of the issue",
      "range": {{ "start_line": N, "start_col": N, "end_line": N, "end_col": N }},
      "severity": "Error | Warning | Info",
      "code": "skill/issue-code"
    }}
  ]
}}

ISSUE TYPES TO DETECT:

1. FRONTMATTER (skill/frontmatter-*)
   - Missing or invalid required fields
   - Unknown or deprecated fields
   - Invalid allowed-tools values

2. INSTRUCTIONS (skill/instruction-*)
   - Vague or ambiguous instructions
   - Contradictory instructions
   - Missing edge case handling

3. SAFETY (skill/safety-*)
   - Overly permissive tool access (e.g., allowing all Bash commands)
   - Missing safety constraints
   - Potential prompt injection vulnerabilities from $ARGUMENTS

4. SUBSTITUTIONS (skill/substitution-*)
   - argument-hint set but $ARGUMENTS not used
   - Unvalidated $ARGUMENTS usage (potential injection)
   - Missing context handling for ${{{{CLAUDE_SESSION_ID}}}}

5. STRUCTURE (skill/structure-*)
   - Skill body too long (>500 lines)
   - Missing progressive disclosure (should reference external docs)
   - Deeply nested instructions

6. RESOURCES (skill/resource-*)
   - Broken references to scripts/references/resources
   - Unused file references

---
SKILL FRONTMATTER:
{frontmatter_json}

SKILL CONTENT:
{content}

SKILL IR:
{ir_json}
---

Respond with only the JSON output, no explanation."#
    )
}

/// Generate the autofix suggestion prompt.
pub fn suggest_fixes_prompt(content: &str, diagnostics_json: &str) -> String {
    format!(
        r#"You are a prompt improvement assistant. For each diagnostic issue, suggest a fix.

DIAGNOSTIC TYPES AND FIXES:

1. prompt/conflict-* → Reword to remove contradiction or add clarification
2. prompt/unreachable-* → Remove unreachable code or adjust conditions
3. prompt/malformed-* → Fix syntax (proper annotation format, matching brackets)
4. prompt/style-* → Improve clarity (be more specific, add examples)
5. prompt/safety-* → Add guardrails (input validation, edge case handling)

OUTPUT FORMAT (JSON):
{{
  "fixes": [
    {{
      "title": "Short description of the fix",
      "diagnostic_code": "prompt/issue-code",
      "edits": [
        {{
          "start_line": 0,
          "start_col": 0,
          "end_line": 0,
          "end_col": 10,
          "new_text": "replacement text"
        }}
      ],
      "is_preferred": true
    }}
  ]
}}

RULES:
1. Line/column numbers are 0-indexed
2. is_preferred=true for safe, obvious fixes
3. is_preferred=false for fixes that change behavior significantly
4. For insertions, use same start/end position
5. For deletions, use empty new_text

---
PROMPT TEMPLATE:
{content}

DIAGNOSTICS:
{diagnostics_json}
---

Respond with only the JSON output, no explanation."#
    )
}

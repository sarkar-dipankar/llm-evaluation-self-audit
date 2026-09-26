//! Semantic tokens for syntax highlighting.

use once_cell::sync::Lazy;
use tower_lsp::lsp_types::{SemanticToken, SemanticTokenType, SemanticTokensLegend};

/// Token types for prompt files.
pub const TOKEN_TYPES: &[SemanticTokenType] = &[
    SemanticTokenType::COMMENT,   // 0: // @rule, // @condition
    SemanticTokenType::KEYWORD,   // 1: {% if %}, {% for %}, YAML keywords
    SemanticTokenType::VARIABLE,  // 2: {{ variable }}, $ARGUMENTS, ${CLAUDE_SESSION_ID}
    SemanticTokenType::STRING,    // 3: plain text, YAML values
    SemanticTokenType::MACRO,     // 4: annotation names, `!command`
    SemanticTokenType::OPERATOR,  // 5: {{ }}, {% %}, ---, :
    SemanticTokenType::PROPERTY,  // 6: YAML keys (name, description, etc.)
    SemanticTokenType::NAMESPACE, // 7: file references (references/, scripts/)
];

/// Semantic tokens legend.
pub static LEGEND: Lazy<SemanticTokensLegend> = Lazy::new(|| SemanticTokensLegend {
    token_types: TOKEN_TYPES.to_vec(),
    token_modifiers: vec![],
});

/// Token type indices.
const TOKEN_COMMENT: u32 = 0;
const TOKEN_KEYWORD: u32 = 1;
const TOKEN_VARIABLE: u32 = 2;
const TOKEN_STRING: u32 = 3;
const TOKEN_MACRO: u32 = 4;
const TOKEN_OPERATOR: u32 = 5;
const TOKEN_PROPERTY: u32 = 6;
const TOKEN_NAMESPACE: u32 = 7;

/// Tokenize document content into semantic tokens.
pub fn tokenize(content: &str) -> Vec<SemanticToken> {
    let mut tokens = Vec::new();
    let mut prev_line = 0u32;
    let mut prev_char = 0u32;

    for (line_num, line) in content.lines().enumerate() {
        let line_num = line_num as u32;
        let trimmed = line.trim();

        // Handle annotation comments
        if trimmed.starts_with("// @") {
            // The entire comment
            let start_col = line.find("//").unwrap_or(0) as u32;

            // Comment prefix
            tokens.push(make_token(
                line_num,
                start_col,
                &mut prev_line,
                &mut prev_char,
                5, // "// @" length
                TOKEN_COMMENT,
            ));

            // Annotation type (rule, condition, breakpoint)
            if let Some(rest) = trimmed.strip_prefix("// @") {
                let parts: Vec<&str> = rest.split_whitespace().collect();
                if !parts.is_empty() {
                    let annotation_type = parts[0];
                    let type_start = start_col + 4; // after "// @"

                    tokens.push(make_token(
                        line_num,
                        type_start,
                        &mut prev_line,
                        &mut prev_char,
                        annotation_type.len() as u32,
                        TOKEN_KEYWORD,
                    ));

                    // Annotation ID (e.g., RULE:NAME)
                    if parts.len() > 1 {
                        let id = parts[1];
                        let id_start = type_start + annotation_type.len() as u32 + 1;

                        tokens.push(make_token(
                            line_num,
                            id_start,
                            &mut prev_line,
                            &mut prev_char,
                            id.len() as u32,
                            TOKEN_MACRO,
                        ));
                    }
                }
            }
            continue;
        }

        // Handle control blocks {% ... %}
        if trimmed.starts_with("{%") {
            let start_col = line.find("{%").unwrap_or(0) as u32;

            // Opening {%
            tokens.push(make_token(
                line_num,
                start_col,
                &mut prev_line,
                &mut prev_char,
                2,
                TOKEN_OPERATOR,
            ));

            // Keyword (if, for, else, endif, endfor)
            if let Some(rest) = trimmed.strip_prefix("{%") {
                if let Some(inner) = rest.strip_suffix("%}") {
                    let inner = inner.trim();
                    let keyword = inner.split_whitespace().next().unwrap_or("");

                    if !keyword.is_empty() {
                        // Find keyword position
                        if let Some(kw_offset) = line.find(keyword) {
                            tokens.push(make_token(
                                line_num,
                                kw_offset as u32,
                                &mut prev_line,
                                &mut prev_char,
                                keyword.len() as u32,
                                TOKEN_KEYWORD,
                            ));
                        }
                    }
                }
            }

            // Closing %}
            if let Some(end_pos) = line.find("%}") {
                tokens.push(make_token(
                    line_num,
                    end_pos as u32,
                    &mut prev_line,
                    &mut prev_char,
                    2,
                    TOKEN_OPERATOR,
                ));
            }
            continue;
        }

        // Handle variable interpolations {{ ... }}
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            if i + 1 < chars.len() && chars[i] == '{' && chars[i + 1] == '{' {
                // Opening {{
                tokens.push(make_token(
                    line_num,
                    i as u32,
                    &mut prev_line,
                    &mut prev_char,
                    2,
                    TOKEN_OPERATOR,
                ));

                // Find variable name
                let mut var_start = i + 2;
                while var_start < chars.len() && chars[var_start].is_whitespace() {
                    var_start += 1;
                }

                let mut var_end = var_start;
                while var_end < chars.len()
                    && chars[var_end] != '}'
                    && !chars[var_end].is_whitespace()
                {
                    var_end += 1;
                }

                if var_end > var_start {
                    tokens.push(make_token(
                        line_num,
                        var_start as u32,
                        &mut prev_line,
                        &mut prev_char,
                        (var_end - var_start) as u32,
                        TOKEN_VARIABLE,
                    ));
                }

                // Find closing }}
                let mut close_pos = var_end;
                while close_pos + 1 < chars.len() {
                    if chars[close_pos] == '}' && chars[close_pos + 1] == '}' {
                        tokens.push(make_token(
                            line_num,
                            close_pos as u32,
                            &mut prev_line,
                            &mut prev_char,
                            2,
                            TOKEN_OPERATOR,
                        ));
                        i = close_pos + 2;
                        break;
                    }
                    close_pos += 1;
                }

                if close_pos + 1 >= chars.len() {
                    i = chars.len();
                }
                continue;
            }

            i += 1;
        }
    }

    tokens
}

/// Tokenize skill file content (SKILL.md with YAML frontmatter).
pub fn tokenize_skill(content: &str) -> Vec<SemanticToken> {
    let mut tokens = Vec::new();
    let mut prev_line = 0u32;
    let mut prev_char = 0u32;

    let lines: Vec<&str> = content.lines().collect();
    let mut in_frontmatter = false;
    let mut frontmatter_ended = false;

    for (line_num, line) in lines.iter().enumerate() {
        let line_num = line_num as u32;
        let trimmed = line.trim();

        // Handle frontmatter delimiters ---
        if trimmed == "---" {
            if !in_frontmatter && !frontmatter_ended {
                in_frontmatter = true;
            } else if in_frontmatter {
                in_frontmatter = false;
                frontmatter_ended = true;
            }

            // Token for ---
            let start = line.find("---").unwrap_or(0) as u32;
            tokens.push(make_token(
                line_num,
                start,
                &mut prev_line,
                &mut prev_char,
                3,
                TOKEN_OPERATOR,
            ));
            continue;
        }

        if in_frontmatter {
            // Parse YAML key: value
            tokenize_yaml_line(line, line_num, &mut tokens, &mut prev_line, &mut prev_char);
        } else if frontmatter_ended {
            // Body: find substitutions, commands, links
            tokenize_skill_body_line(line, line_num, &mut tokens, &mut prev_line, &mut prev_char);
        }
    }

    tokens
}

/// Tokenize a YAML frontmatter line.
fn tokenize_yaml_line(
    line: &str,
    line_num: u32,
    tokens: &mut Vec<SemanticToken>,
    prev_line: &mut u32,
    prev_char: &mut u32,
) {
    let trimmed = line.trim();

    // Skip comments and empty lines
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return;
    }

    // Handle list items (- value)
    if trimmed.starts_with("- ") {
        if let Some(dash_pos) = line.find('-') {
            // Dash operator
            tokens.push(make_token(
                line_num,
                dash_pos as u32,
                prev_line,
                prev_char,
                1,
                TOKEN_OPERATOR,
            ));

            // Value after dash
            let value_start = dash_pos + 2;
            if value_start < line.len() {
                let value = &line[value_start..];
                tokens.push(make_token(
                    line_num,
                    value_start as u32,
                    prev_line,
                    prev_char,
                    value.trim_end().len() as u32,
                    TOKEN_STRING,
                ));
            }
        }
        return;
    }

    // Handle key: value pairs
    if let Some(colon_pos) = trimmed.find(':') {
        let line_colon_pos = line.find(':').unwrap_or(0);
        let key = &trimmed[..colon_pos];

        // Key token
        let key_start = line.find(key).unwrap_or(0) as u32;
        tokens.push(make_token(
            line_num,
            key_start,
            prev_line,
            prev_char,
            key.len() as u32,
            TOKEN_PROPERTY,
        ));

        // Colon operator
        tokens.push(make_token(
            line_num,
            line_colon_pos as u32,
            prev_line,
            prev_char,
            1,
            TOKEN_OPERATOR,
        ));

        // Value (if on same line)
        let value = trimmed[colon_pos + 1..].trim();
        if !value.is_empty() {
            let value_start = line_colon_pos
                + 1
                + (line.len() - line_colon_pos - 1 - line[line_colon_pos + 1..].trim_start().len());
            tokens.push(make_token(
                line_num,
                value_start as u32,
                prev_line,
                prev_char,
                value.len() as u32,
                TOKEN_STRING,
            ));
        }
    }
}

/// Tokenize a skill body line (after frontmatter).
fn tokenize_skill_body_line(
    line: &str,
    line_num: u32,
    tokens: &mut Vec<SemanticToken>,
    prev_line: &mut u32,
    prev_char: &mut u32,
) {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        // $ARGUMENTS substitution
        if i + 10 <= chars.len() {
            let slice: String = chars[i..i + 10].iter().collect();
            if slice.starts_with("$ARGUMENTS") {
                tokens.push(make_token(
                    line_num,
                    i as u32,
                    prev_line,
                    prev_char,
                    10,
                    TOKEN_VARIABLE,
                ));
                i += 10;
                continue;
            }
        }

        // ${CLAUDE_SESSION_ID} substitution
        if i + 20 <= chars.len() {
            let slice: String = chars[i..i + 20].iter().collect();
            if slice.starts_with("${CLAUDE_SESSION_ID}") {
                tokens.push(make_token(
                    line_num,
                    i as u32,
                    prev_line,
                    prev_char,
                    20,
                    TOKEN_VARIABLE,
                ));
                i += 20;
                continue;
            }
        }

        // `!command` execution
        if chars[i] == '`' && i + 1 < chars.len() && chars[i + 1] == '!' {
            let start = i;
            i += 2; // skip `!

            // Find closing backtick
            while i < chars.len() && chars[i] != '`' {
                i += 1;
            }

            if i < chars.len() {
                i += 1; // include closing backtick
                tokens.push(make_token(
                    line_num,
                    start as u32,
                    prev_line,
                    prev_char,
                    (i - start) as u32,
                    TOKEN_MACRO,
                ));
            }
            continue;
        }

        // [text](path) markdown links
        if chars[i] == '[' {
            let start = i;
            i += 1;

            // Find ]
            while i < chars.len() && chars[i] != ']' {
                i += 1;
            }

            if i < chars.len() && i + 1 < chars.len() && chars[i + 1] == '(' {
                i += 2; // skip ](

                // Find )
                let path_start = i;
                while i < chars.len() && chars[i] != ')' {
                    i += 1;
                }

                if i < chars.len() {
                    // Check if it's a file reference (scripts/, references/, resources/)
                    let path: String = chars[path_start..i].iter().collect();
                    if path.starts_with("scripts/")
                        || path.starts_with("references/")
                        || path.starts_with("resources/")
                        || path.ends_with(".md")
                        || path.ends_with(".py")
                        || path.ends_with(".sh")
                    {
                        tokens.push(make_token(
                            line_num,
                            start as u32,
                            prev_line,
                            prev_char,
                            (i + 1 - start) as u32,
                            TOKEN_NAMESPACE,
                        ));
                    }
                    i += 1; // skip )
                }
            }
            continue;
        }

        i += 1;
    }
}

/// Create a semantic token with delta encoding.
fn make_token(
    line: u32,
    char: u32,
    prev_line: &mut u32,
    prev_char: &mut u32,
    length: u32,
    token_type: u32,
) -> SemanticToken {
    let delta_line = line - *prev_line;
    let delta_start = if delta_line == 0 {
        char - *prev_char
    } else {
        char
    };

    *prev_line = line;
    *prev_char = char;

    SemanticToken {
        delta_line,
        delta_start,
        length,
        token_type,
        token_modifiers_bitset: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize_annotation() {
        let content = "// @rule RULE:GREETING";
        let tokens = tokenize(content);
        assert!(!tokens.is_empty());
    }

    #[test]
    fn test_tokenize_variable() {
        let content = "Hello {{ name }}!";
        let tokens = tokenize(content);
        // Should have tokens for {{ and name and }}
        assert!(tokens.len() >= 2);
    }

    #[test]
    fn test_tokenize_control_block() {
        let content = "{% if condition %}";
        let tokens = tokenize(content);
        assert!(!tokens.is_empty());
    }

    #[test]
    fn test_tokenize_skill_frontmatter() {
        let content = r#"---
name: test-skill
description: A test skill
allowed-tools:
  - Read
  - Grep
---
Search for $ARGUMENTS"#;
        let tokens = tokenize_skill(content);
        // Should have tokens for ---, name:, description:, etc.
        assert!(!tokens.is_empty());
    }

    #[test]
    fn test_tokenize_skill_substitutions() {
        let content = r#"---
name: test
description: test
---
Search for $ARGUMENTS
Session: ${CLAUDE_SESSION_ID}"#;
        let tokens = tokenize_skill(content);
        // Should have tokens for $ARGUMENTS and ${CLAUDE_SESSION_ID}
        assert!(!tokens.is_empty());
    }

    #[test]
    fn test_tokenize_skill_command() {
        let content = r#"---
name: test
description: test
---
Current status: `!git status`"#;
        let tokens = tokenize_skill(content);
        // Should have a token for `!git status`
        assert!(!tokens.is_empty());
    }
}

//! Language Server Protocol implementation for PromptDbg.
//!
//! Provides:
//! - Document symbols (outline view)
//! - Semantic tokens (syntax highlighting)
//! - Diagnostics (linting)
//! - Code lenses (Debug, Run Tests)
//! - Custom prompt/* methods

use std::collections::HashMap;
use std::sync::Arc;

use prompt_ir::{IRKind, IRNode, PromptDiagnostic, PromptIR, TextRange};
use prompt_template::skill::{
    detect_document_kind, parse_skill, validate_frontmatter, DocumentKind,
};
use prompt_template::{parse_annotations, AnnotationKind};
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

mod semantic_tokens;

pub use semantic_tokens::LEGEND;

/// Document state cached by the LSP server.
#[derive(Debug, Clone)]
struct DocumentState {
    /// Document content
    content: String,
    /// Document kind (template, skill, or plain prompt)
    kind: DocumentKind,
    /// Cached IR (if generated)
    ir: Option<PromptIR>,
}

/// The PromptDbg language server.
pub struct PromptDbgServer {
    client: Client,
    documents: Arc<RwLock<HashMap<Url, DocumentState>>>,
}

impl PromptDbgServer {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            documents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Generate IR from document content using local parsing.
    /// In production, this would call the SLM analysis client.
    async fn generate_ir(&self, uri: &Url, content: &str, version: i32) -> PromptIR {
        let annotations = parse_annotations(content);

        let nodes: Vec<IRNode> = annotations
            .into_iter()
            .map(|ann| {
                let kind = match ann.kind {
                    AnnotationKind::Rule => IRKind::Rule,
                    AnnotationKind::Condition => IRKind::Condition,
                    AnnotationKind::Breakpoint => IRKind::Breakpoint,
                    AnnotationKind::Provider => IRKind::Directive,
                };

                IRNode {
                    id: ann.id,
                    kind,
                    label: None,
                    range: ann.range,
                    meta: Default::default(),
                }
            })
            .collect();

        PromptIR {
            uri: uri.to_string(),
            version,
            nodes,
            meta: None,
        }
    }

    /// Generate IR from a parsed skill file.
    fn generate_skill_ir(
        &self,
        uri: &Url,
        skill: &prompt_template::skill::ParsedSkill,
        version: i32,
    ) -> PromptIR {
        let mut nodes = Vec::new();

        // Add constraint nodes from frontmatter
        if !skill.frontmatter.allowed_tools.is_empty() {
            nodes.push(IRNode {
                id: "CONSTRAINT:TOOL_RESTRICTION".to_string(),
                kind: IRKind::SkillConstraint,
                label: Some(format!(
                    "Allowed tools: {}",
                    skill.frontmatter.allowed_tools.join(", ")
                )),
                range: skill.frontmatter_range,
                meta: Default::default(),
            });
        }

        if skill.frontmatter.disable_model_invocation {
            nodes.push(IRNode {
                id: "CONSTRAINT:USER_ONLY".to_string(),
                kind: IRKind::SkillConstraint,
                label: Some("Can only be invoked by user".to_string()),
                range: skill.frontmatter_range,
                meta: Default::default(),
            });
        }

        // Add substitution nodes
        for (i, loc) in skill.arguments_locations.iter().enumerate() {
            nodes.push(IRNode {
                id: format!("SUBSTITUTION:ARGUMENTS_{}", i),
                kind: IRKind::SkillInstruction,
                label: Some("$ARGUMENTS substitution".to_string()),
                range: loc.range,
                meta: Default::default(),
            });
        }

        for (i, loc) in skill.session_id_locations.iter().enumerate() {
            nodes.push(IRNode {
                id: format!("SUBSTITUTION:SESSION_ID_{}", i),
                kind: IRKind::SkillInstruction,
                label: Some("${CLAUDE_SESSION_ID} substitution".to_string()),
                range: loc.range,
                meta: Default::default(),
            });
        }

        // Add command execution nodes
        for (i, cmd) in skill.command_executions.iter().enumerate() {
            nodes.push(IRNode {
                id: format!("COMMAND:EXEC_{}", i),
                kind: IRKind::SkillInstruction,
                label: Some(format!("`!{}` command", cmd.command)),
                range: cmd.range,
                meta: Default::default(),
            });
        }

        PromptIR {
            uri: uri.to_string(),
            version,
            nodes,
            meta: None,
        }
    }

    /// Convert IR nodes to LSP document symbols.
    fn ir_to_symbols(&self, ir: &PromptIR) -> Vec<DocumentSymbol> {
        ir.nodes
            .iter()
            .map(|node| {
                let kind = match node.kind {
                    IRKind::Rule => SymbolKind::METHOD,
                    IRKind::Condition => SymbolKind::EVENT,
                    IRKind::Breakpoint => SymbolKind::BOOLEAN,
                    IRKind::Section => SymbolKind::NAMESPACE,
                    IRKind::Directive => SymbolKind::PROPERTY,
                    IRKind::SkillInstruction => SymbolKind::FUNCTION,
                    IRKind::SkillConstraint => SymbolKind::CONSTANT,
                };

                let range = self.text_range_to_lsp(&node.range);

                #[allow(deprecated)]
                DocumentSymbol {
                    name: node.id.clone(),
                    detail: node.label.clone(),
                    kind,
                    tags: None,
                    deprecated: None,
                    range,
                    selection_range: range,
                    children: None,
                }
            })
            .collect()
    }

    /// Convert TextRange to LSP Range.
    fn text_range_to_lsp(&self, range: &TextRange) -> Range {
        Range {
            start: Position {
                line: range.start_line,
                character: range.start_col,
            },
            end: Position {
                line: range.end_line,
                character: range.end_col,
            },
        }
    }

    /// Convert PromptDiagnostic to LSP Diagnostic.
    fn prompt_diagnostic_to_lsp(&self, diag: &PromptDiagnostic) -> Diagnostic {
        Diagnostic {
            range: self.text_range_to_lsp(&diag.range),
            severity: Some(match diag.severity {
                prompt_ir::DiagnosticSeverity::Error => DiagnosticSeverity::ERROR,
                prompt_ir::DiagnosticSeverity::Warning => DiagnosticSeverity::WARNING,
                prompt_ir::DiagnosticSeverity::Info => DiagnosticSeverity::INFORMATION,
            }),
            code: diag.code.clone().map(NumberOrString::String),
            source: Some("promptdbg".to_string()),
            message: diag.message.clone(),
            related_information: None,
            tags: None,
            code_description: None,
            data: None,
        }
    }

    /// Generate code lenses for a document.
    fn generate_code_lenses(&self, uri: &Url, ir: &PromptIR) -> Vec<CodeLens> {
        let mut lenses = Vec::new();

        // Add "Debug Prompt" lens at the top
        lenses.push(CodeLens {
            range: Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 0,
                    character: 0,
                },
            },
            command: Some(Command {
                title: "▶ Debug Prompt".to_string(),
                command: "promptdbg.debug".to_string(),
                arguments: Some(vec![serde_json::json!(uri.to_string())]),
            }),
            data: None,
        });

        // Add lens for each rule
        for node in ir.rules() {
            let range = self.text_range_to_lsp(&node.range);

            lenses.push(CodeLens {
                range,
                command: Some(Command {
                    title: "Run Tests".to_string(),
                    command: "promptdbg.runTests".to_string(),
                    arguments: Some(vec![
                        serde_json::json!(uri.to_string()),
                        serde_json::json!(node.id),
                    ]),
                }),
                data: None,
            });
        }

        lenses
    }

    /// Validate template syntax and return diagnostics.
    fn validate_template(&self, content: &str) -> Vec<PromptDiagnostic> {
        let mut diagnostics = Vec::new();

        // Check for unmatched {{ }}
        let mut in_var = false;
        let mut var_start_line = 0;
        let mut var_start_col = 0;

        for (line_num, line) in content.lines().enumerate() {
            let chars: Vec<char> = line.chars().collect();
            let mut i = 0;

            while i < chars.len() {
                if i + 1 < chars.len() && chars[i] == '{' && chars[i + 1] == '{' {
                    if in_var {
                        diagnostics.push(PromptDiagnostic::error(
                            "Nested {{ not allowed",
                            TextRange::new(
                                line_num as u32,
                                i as u32,
                                line_num as u32,
                                i as u32 + 2,
                            ),
                        ));
                    }
                    in_var = true;
                    var_start_line = line_num as u32;
                    var_start_col = i as u32;
                    i += 2;
                    continue;
                }

                if i + 1 < chars.len() && chars[i] == '}' && chars[i + 1] == '}' {
                    if !in_var {
                        diagnostics.push(PromptDiagnostic::error(
                            "Unexpected }}",
                            TextRange::new(
                                line_num as u32,
                                i as u32,
                                line_num as u32,
                                i as u32 + 2,
                            ),
                        ));
                    }
                    in_var = false;
                    i += 2;
                    continue;
                }

                i += 1;
            }
        }

        if in_var {
            diagnostics.push(PromptDiagnostic::error(
                "Unclosed {{",
                TextRange::new(
                    var_start_line,
                    var_start_col,
                    var_start_line,
                    var_start_col + 2,
                ),
            ));
        }

        // Check for unmatched {% %}
        let mut block_stack: Vec<(u32, u32, &str)> = Vec::new();

        for (line_num, line) in content.lines().enumerate() {
            let trimmed = line.trim();

            if let Some(rest) = trimmed.strip_prefix("{%") {
                if let Some(inner) = rest.strip_suffix("%}") {
                    let inner = inner.trim();

                    if inner.starts_with("if ") {
                        block_stack.push((line_num as u32, 0, "if"));
                    } else if inner.starts_with("for ") {
                        block_stack.push((line_num as u32, 0, "for"));
                    } else if inner == "else" {
                        if block_stack.is_empty() || block_stack.last().map(|b| b.2) != Some("if") {
                            diagnostics.push(PromptDiagnostic::error(
                                "Unexpected {% else %}",
                                TextRange::new(
                                    line_num as u32,
                                    0,
                                    line_num as u32,
                                    line.len() as u32,
                                ),
                            ));
                        }
                    } else if inner == "endif" {
                        if block_stack.is_empty() || block_stack.last().map(|b| b.2) != Some("if") {
                            diagnostics.push(PromptDiagnostic::error(
                                "Unexpected {% endif %}",
                                TextRange::new(
                                    line_num as u32,
                                    0,
                                    line_num as u32,
                                    line.len() as u32,
                                ),
                            ));
                        } else {
                            block_stack.pop();
                        }
                    } else if inner == "endfor" {
                        if block_stack.is_empty() || block_stack.last().map(|b| b.2) != Some("for")
                        {
                            diagnostics.push(PromptDiagnostic::error(
                                "Unexpected {% endfor %}",
                                TextRange::new(
                                    line_num as u32,
                                    0,
                                    line_num as u32,
                                    line.len() as u32,
                                ),
                            ));
                        } else {
                            block_stack.pop();
                        }
                    }
                }
            }
        }

        for (line, col, block_type) in block_stack {
            diagnostics.push(PromptDiagnostic::error(
                format!("Unclosed {{% {} %}}", block_type),
                TextRange::new(line, col, line, col + 10),
            ));
        }

        diagnostics
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for PromptDbgServer {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                document_symbol_provider: Some(OneOf::Left(true)),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            legend: LEGEND.clone(),
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                            range: Some(false),
                            ..Default::default()
                        },
                    ),
                ),
                code_lens_provider: Some(CodeLensOptions {
                    resolve_provider: Some(false),
                }),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "promptdbg".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "PromptDbg LSP initialized")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let content = params.text_document.text;
        let version = params.text_document.version;

        // Detect document kind from filename
        let filename = uri.path().rsplit('/').next().unwrap_or("");
        let kind = detect_document_kind(filename, &content);

        // Generate IR and diagnostics based on document kind
        let (ir, diagnostics) = match kind {
            DocumentKind::Skill => {
                // Parse and validate skill file
                let mut diags = Vec::new();
                let ir = match parse_skill(&content) {
                    Ok(skill) => {
                        // Validate frontmatter
                        for fm_diag in validate_frontmatter(&skill) {
                            diags.push(PromptDiagnostic {
                                message: fm_diag.message,
                                range: fm_diag.range,
                                severity: match fm_diag.severity {
                                    prompt_template::skill::DiagnosticSeverity::Error => {
                                        prompt_ir::DiagnosticSeverity::Error
                                    }
                                    prompt_template::skill::DiagnosticSeverity::Warning => {
                                        prompt_ir::DiagnosticSeverity::Warning
                                    }
                                    prompt_template::skill::DiagnosticSeverity::Info => {
                                        prompt_ir::DiagnosticSeverity::Info
                                    }
                                },
                                code: Some(fm_diag.code),
                            });
                        }
                        // Generate basic IR from skill structure
                        self.generate_skill_ir(&uri, &skill, version)
                    }
                    Err(e) => {
                        diags.push(PromptDiagnostic::error(
                            format!("Failed to parse skill: {}", e),
                            TextRange::new(0, 0, 0, 1),
                        ));
                        PromptIR {
                            uri: uri.to_string(),
                            version,
                            nodes: vec![],
                            meta: None,
                        }
                    }
                };
                (ir, diags)
            }
            DocumentKind::Template | DocumentKind::PlainPrompt => {
                // Generate IR from annotations
                let ir = self.generate_ir(&uri, &content, version).await;
                // Validate template syntax
                let diags = self.validate_template(&content);
                (ir, diags)
            }
        };

        // Store document state
        {
            let mut docs = self.documents.write().await;
            docs.insert(
                uri.clone(),
                DocumentState {
                    content,
                    kind,
                    ir: Some(ir),
                },
            );
        }

        // Publish diagnostics
        let lsp_diags: Vec<Diagnostic> = diagnostics
            .iter()
            .map(|d| self.prompt_diagnostic_to_lsp(d))
            .collect();

        self.client
            .publish_diagnostics(uri, lsp_diags, Some(version))
            .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        let version = params.text_document.version;

        if let Some(change) = params.content_changes.into_iter().last() {
            let content = change.text;

            // Detect document kind from filename
            let filename = uri.path().rsplit('/').next().unwrap_or("");
            let kind = detect_document_kind(filename, &content);

            // Generate IR and diagnostics based on document kind
            let (ir, diagnostics) = match kind {
                DocumentKind::Skill => {
                    let mut diags = Vec::new();
                    let ir = match parse_skill(&content) {
                        Ok(skill) => {
                            for fm_diag in validate_frontmatter(&skill) {
                                diags.push(PromptDiagnostic {
                                    message: fm_diag.message,
                                    range: fm_diag.range,
                                    severity: match fm_diag.severity {
                                        prompt_template::skill::DiagnosticSeverity::Error => {
                                            prompt_ir::DiagnosticSeverity::Error
                                        }
                                        prompt_template::skill::DiagnosticSeverity::Warning => {
                                            prompt_ir::DiagnosticSeverity::Warning
                                        }
                                        prompt_template::skill::DiagnosticSeverity::Info => {
                                            prompt_ir::DiagnosticSeverity::Info
                                        }
                                    },
                                    code: Some(fm_diag.code),
                                });
                            }
                            self.generate_skill_ir(&uri, &skill, version)
                        }
                        Err(e) => {
                            diags.push(PromptDiagnostic::error(
                                format!("Failed to parse skill: {}", e),
                                TextRange::new(0, 0, 0, 1),
                            ));
                            PromptIR {
                                uri: uri.to_string(),
                                version,
                                nodes: vec![],
                                meta: None,
                            }
                        }
                    };
                    (ir, diags)
                }
                DocumentKind::Template | DocumentKind::PlainPrompt => {
                    let ir = self.generate_ir(&uri, &content, version).await;
                    let diags = self.validate_template(&content);
                    (ir, diags)
                }
            };

            // Store document state
            {
                let mut docs = self.documents.write().await;
                docs.insert(
                    uri.clone(),
                    DocumentState {
                        content,
                        kind,
                        ir: Some(ir),
                    },
                );
            }

            // Publish diagnostics
            let lsp_diags: Vec<Diagnostic> = diagnostics
                .iter()
                .map(|d| self.prompt_diagnostic_to_lsp(d))
                .collect();

            self.client
                .publish_diagnostics(uri, lsp_diags, Some(version))
                .await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let mut docs = self.documents.write().await;
        docs.remove(&params.text_document.uri);
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let docs = self.documents.read().await;

        if let Some(state) = docs.get(&params.text_document.uri) {
            if let Some(ir) = &state.ir {
                let symbols = self.ir_to_symbols(ir);
                return Ok(Some(DocumentSymbolResponse::Nested(symbols)));
            }
        }

        Ok(None)
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let docs = self.documents.read().await;

        if let Some(state) = docs.get(&params.text_document.uri) {
            // Use appropriate tokenizer based on document kind
            let tokens = match state.kind {
                DocumentKind::Skill => semantic_tokens::tokenize_skill(&state.content),
                DocumentKind::Template | DocumentKind::PlainPrompt => {
                    semantic_tokens::tokenize(&state.content)
                }
            };
            return Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
                result_id: None,
                data: tokens,
            })));
        }

        Ok(None)
    }

    async fn code_lens(&self, params: CodeLensParams) -> Result<Option<Vec<CodeLens>>> {
        let docs = self.documents.read().await;

        if let Some(state) = docs.get(&params.text_document.uri) {
            if let Some(ir) = &state.ir {
                let lenses = self.generate_code_lenses(&params.text_document.uri, ir);
                return Ok(Some(lenses));
            }
        }

        Ok(Some(Vec::new()))
    }
}

/// Run the LSP server on stdin/stdout.
pub async fn run_server() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(PromptDbgServer::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}

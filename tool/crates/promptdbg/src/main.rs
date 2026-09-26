//! PromptDbg CLI - Debug your LLM prompts
//!
//! Usage:
//!   promptdbg run <file.prompt.rtpl>           Run a prompt file
//!   promptdbg debug <file.prompt.rtpl>         Debug a prompt file
//!   promptdbg analyze <file.prompt.rtpl>       Generate IR and lint
//!   promptdbg render <file.prompt.rtpl>        Render template only
//!   promptdbg test <file.prompt.rtpl>          Generate test cases
//!   promptdbg breakpoints <file.prompt.rtpl>   Suggest breakpoint locations
//!   promptdbg lsp                              Start Language Server
//!   promptdbg dap                              Start Debug Adapter

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use prompt_config::auto_detect_provider;
use prompt_ir::AnalysisConfig;
use prompt_runtime::{DebugConfig, SkillExecutionConfig, SkillExecutor, SkillStepKind};
use prompt_slm::{AnalysisClient, AnalysisProvider};
use prompt_template::skill::{parse_skill, validate_frontmatter};
use prompt_template::skill_directory::SkillDirectory;
use serde_json::Value;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

#[derive(Parser)]
#[command(name = "promptdbg")]
#[command(about = "Debug your LLM prompts", long_about = None)]
struct Cli {
    /// Enable verbose output
    #[arg(short, long)]
    verbose: bool,

    /// Persist the IR + response cache to this directory for deterministic replay
    /// and cost reduction across runs (memory-only if omitted).
    #[arg(long, global = true)]
    cache_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

/// How to build the prompt IR.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
enum IrEngine {
    /// Deterministic: build IR from explicit `// @rule`/`@condition`/`@breakpoint`
    /// annotations only. No LLM, no API key, byte-identical output across runs.
    Annotation,
    /// LLM-backed inference via the analysis provider (non-deterministic).
    #[default]
    Llm,
}

/// Kill-detection oracle for mutation testing (used in the oracle ablation).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
enum OracleKind {
    /// Kill when any observable behavior differs (output, covered rules, branches).
    #[default]
    Trace,
    /// Kill only when the rendered output text differs.
    Output,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a prompt file
    Run {
        /// Path to the prompt file
        file: PathBuf,

        /// JSON context for template variables
        #[arg(short, long)]
        context: Option<String>,

        /// Context file (JSON)
        #[arg(long)]
        context_file: Option<PathBuf>,
    },

    /// Debug a prompt file (with breakpoints)
    Debug {
        /// Path to the prompt file
        file: PathBuf,

        /// JSON context for template variables
        #[arg(short, long)]
        context: Option<String>,

        /// Template-only mode (no LLM call)
        #[arg(long)]
        template_only: bool,
    },

    /// Analyze a prompt file (generate IR and lint)
    Analyze {
        /// Path to the prompt file
        file: PathBuf,

        /// Output format (json, pretty)
        #[arg(short, long, default_value = "pretty")]
        format: String,

        /// IR engine: `llm` (inferred, default) or `annotation` (deterministic, no provider)
        #[arg(long, value_enum, default_value_t = IrEngine::Llm)]
        engine: IrEngine,

        /// Skip linting (emit IR only) — avoids a second LLM call per invocation
        #[arg(long)]
        no_lint: bool,

        /// Ablation: lint against an EMPTY IR (no structural context) to measure the
        /// contribution of the IR to the linter's findings
        #[arg(long)]
        ablate_ir: bool,

        /// Skip the deterministic static lint (e.g. unreachable-rule), leaving only
        /// the LLM diagnostics — used to isolate the LLM linter in studies
        #[arg(long)]
        no_static_lint: bool,
    },

    /// Render a prompt template (no LLM call)
    Render {
        /// Path to the prompt file
        file: PathBuf,

        /// JSON context for template variables
        #[arg(short, long)]
        context: Option<String>,

        /// Context file (JSON)
        #[arg(long)]
        context_file: Option<PathBuf>,
    },

    /// Show detected provider configuration
    Config,

    /// Measure how a set of input contexts covers a prompt's structure
    Coverage {
        /// Path to the prompt file
        file: PathBuf,

        /// JSON file with an array of context objects (defaults to a single empty context)
        #[arg(long)]
        contexts: Option<PathBuf>,

        /// IR engine: `annotation` (deterministic, default) or `llm` (inferred)
        #[arg(long, value_enum, default_value_t = IrEngine::Annotation)]
        engine: IrEngine,

        /// Output format (json, pretty)
        #[arg(short, long, default_value = "pretty")]
        format: String,
    },

    /// Generate input contexts that exercise a prompt's branches (deterministic)
    GenContexts {
        /// Path to the prompt file
        file: PathBuf,

        /// Write the contexts JSON array here (prints to stdout if omitted)
        #[arg(long)]
        out: Option<PathBuf>,
    },

    /// Run mutation testing on a prompt (mutation score over input contexts)
    Mutate {
        /// Path to the prompt file
        file: PathBuf,

        /// JSON file with an array of context objects (defaults to a single empty context)
        #[arg(long)]
        contexts: Option<PathBuf>,

        /// IR engine: `annotation` (deterministic, default) or `llm` (inferred)
        #[arg(long, value_enum, default_value_t = IrEngine::Annotation)]
        engine: IrEngine,

        /// Only list the generated mutants; do not run the suite
        #[arg(long)]
        list: bool,

        /// Kill-detection oracle: `trace` (default) or `output` (output-only ablation)
        #[arg(long, value_enum, default_value_t = OracleKind::Trace)]
        oracle: OracleKind,

        /// Output format (json, pretty)
        #[arg(short, long, default_value = "pretty")]
        format: String,
    },

    /// Batch-measure coverage (and optionally mutation) over a corpus directory
    Bench {
        /// Directory of `.prompt.rtpl` files (searched recursively)
        dir: PathBuf,

        /// JSON file with an array of context objects, shared across all prompts
        #[arg(long)]
        contexts: Option<PathBuf>,

        /// Directory of per-prompt contexts: for `foo.prompt.rtpl`, uses
        /// `<dir>/foo.contexts.json` when present (falls back to --contexts)
        #[arg(long)]
        contexts_dir: Option<PathBuf>,

        /// Also run mutation testing per prompt
        #[arg(long)]
        mutation: bool,

        /// Output format (csv, json)
        #[arg(short, long, default_value = "csv")]
        format: String,
    },

    /// Generate test cases for a prompt
    Test {
        /// Path to the prompt file
        file: PathBuf,

        /// Target node ID (optional, tests all rules if not specified)
        #[arg(short, long)]
        node: Option<String>,

        /// Output format (json, pretty)
        #[arg(short, long, default_value = "pretty")]
        format: String,
    },

    /// Suggest breakpoint locations
    Breakpoints {
        /// Path to the prompt file
        file: PathBuf,

        /// Output format (json, pretty)
        #[arg(short, long, default_value = "pretty")]
        format: String,
    },

    /// Suggest and optionally apply fixes for lint issues
    Fix {
        /// Path to the prompt file
        file: PathBuf,

        /// Apply fixes automatically (default: preview only)
        #[arg(long)]
        apply: bool,

        /// Output format (json, pretty)
        #[arg(short, long, default_value = "pretty")]
        format: String,
    },

    /// Start the Language Server Protocol server
    Lsp,

    /// Start the Debug Adapter Protocol server
    Dap,

    /// Analyze a SKILL.md file
    AnalyzeSkill {
        /// Path to the SKILL.md file
        file: PathBuf,

        /// Output format (json, pretty)
        #[arg(short, long, default_value = "pretty")]
        format: String,
    },

    /// Validate a skill directory (checks SKILL.md and references)
    ValidateSkill {
        /// Path to directory containing SKILL.md
        dir: PathBuf,

        /// Output format (json, pretty)
        #[arg(short, long, default_value = "pretty")]
        format: String,
    },

    /// Run a skill with arguments
    RunSkill {
        /// Path to the skill directory or SKILL.md file
        path: PathBuf,

        /// Arguments to pass to the skill ($ARGUMENTS substitution)
        #[arg(short, long, default_value = "")]
        args: String,

        /// Session ID (auto-generated if not provided)
        #[arg(long)]
        session_id: Option<String>,

        /// Disable command execution
        #[arg(long)]
        no_commands: bool,
    },

    /// Debug a skill with tracing
    DebugSkill {
        /// Path to the skill directory or SKILL.md file
        path: PathBuf,

        /// Arguments to pass to the skill
        #[arg(short, long, default_value = "")]
        args: String,

        /// Template-only mode (no LLM call)
        #[arg(long)]
        template_only: bool,

        /// Output format (pretty, json, trace)
        #[arg(short, long, default_value = "pretty")]
        format: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Setup logging
    let level = if cli.verbose {
        Level::DEBUG
    } else {
        Level::INFO
    };
    // Logs go to stderr so that stdout carries only command output (e.g. `--format json`),
    // keeping machine-readable output clean and byte-reproducible across runs.
    let subscriber = FmtSubscriber::builder()
        .with_max_level(level)
        .with_target(false)
        .with_writer(std::io::stderr)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    // Initialize a persistent cache when requested, so repeated experiment runs can
    // replay IR and provider responses from disk.
    if let Some(dir) = &cli.cache_dir {
        prompt_runtime::init_global_cache(prompt_runtime::CacheConfig {
            cache_dir: Some(dir.clone()),
            ..Default::default()
        });
    }

    let result = match cli.command {
        Commands::Run {
            file,
            context,
            context_file,
        } => run_prompt(&file, context, context_file).await,
        Commands::Debug {
            file,
            context,
            template_only,
        } => debug_prompt(&file, context, template_only).await,
        Commands::Analyze {
            file,
            format,
            engine,
            no_lint,
            ablate_ir,
            no_static_lint,
        } => analyze_prompt(&file, &format, engine, no_lint, ablate_ir, no_static_lint).await,
        Commands::Render {
            file,
            context,
            context_file,
        } => render_prompt(&file, context, context_file),
        Commands::Config => show_config(),
        Commands::Coverage {
            file,
            contexts,
            engine,
            format,
        } => coverage_cmd(&file, contexts.as_deref(), engine, &format).await,
        Commands::GenContexts { file, out } => gen_contexts_cmd(&file, out.as_deref()),
        Commands::Mutate {
            file,
            contexts,
            engine,
            list,
            oracle,
            format,
        } => mutate_cmd(&file, contexts.as_deref(), engine, list, oracle, &format).await,
        Commands::Bench {
            dir,
            contexts,
            contexts_dir,
            mutation,
            format,
        } => {
            bench_cmd(
                &dir,
                contexts.as_deref(),
                contexts_dir.as_deref(),
                mutation,
                &format,
            )
            .await
        }
        Commands::Test { file, node, format } => {
            generate_tests(&file, node.as_deref(), &format).await
        }
        Commands::Breakpoints { file, format } => suggest_breakpoints(&file, &format).await,
        Commands::Fix {
            file,
            apply,
            format,
        } => suggest_fixes(&file, apply, &format).await,
        Commands::Lsp => run_lsp().await,
        Commands::Dap => run_dap().await,
        Commands::AnalyzeSkill { file, format } => analyze_skill(&file, &format).await,
        Commands::ValidateSkill { dir, format } => validate_skill(&dir, &format),
        Commands::RunSkill {
            path,
            args,
            session_id,
            no_commands,
        } => run_skill(&path, &args, session_id, no_commands).await,
        Commands::DebugSkill {
            path,
            args,
            template_only,
            format,
        } => debug_skill(&path, &args, template_only, &format).await,
    };

    // Flush the cache to disk when persistence was requested.
    if cli.cache_dir.is_some() {
        let _ = prompt_runtime::global_cache().persist();
    }
    result
}

async fn run_prompt(
    file: &PathBuf,
    context: Option<String>,
    context_file: Option<PathBuf>,
) -> Result<()> {
    let content = std::fs::read_to_string(file)
        .with_context(|| format!("Failed to read file: {}", file.display()))?;

    let ctx = load_context(context, context_file)?;

    // Get provider config
    let provider_config = auto_detect_provider().ok_or_else(|| {
        anyhow::anyhow!("No provider configured. Set OPENAI_API_KEY or ANTHROPIC_API_KEY")
    })?;

    info!(
        "Using provider: {} ({})",
        provider_config.provider, provider_config.model
    );

    // Generate IR
    let analysis_config = AnalysisConfig::from(&provider_config);
    let analysis_client = AnalysisClient::new(analysis_config)?;
    let ir = generate_ir_cached(&analysis_client, &content, &file.display().to_string()).await?;

    info!("Generated IR with {} nodes", ir.nodes.len());

    // Execute
    let debug_config = DebugConfig {
        context: ctx,
        breakpoints: Vec::new(),
        template_only: false,
        stop_on_entry: false,
    };

    let trace = prompt_runtime::execute(&content, &ir, &provider_config, &debug_config).await?;

    if let Some(answer) = &trace.final_answer {
        println!("\n{}", answer);
    }

    Ok(())
}

async fn debug_prompt(file: &PathBuf, context: Option<String>, template_only: bool) -> Result<()> {
    let content = std::fs::read_to_string(file)
        .with_context(|| format!("Failed to read file: {}", file.display()))?;

    let ctx = load_context(context, None)?;

    if template_only {
        let result = prompt_runtime::execute_template_only(&content, &ctx)?;

        println!("=== Rendered Template ===\n{}\n", result.output);

        println!("=== Variables Used ({}) ===", result.variables_used.len());
        for access in &result.variable_accesses {
            println!(
                "  {} = {} ({})",
                access.name, access.value, access.value_type
            );
            println!("    at line {}:{}", access.line + 1, access.column);
        }

        println!(
            "\n=== Branches Evaluated ({}) ===",
            result.branches_taken.len()
        );
        for branch in &result.branches_taken {
            let status = if branch.taken { "✓" } else { "✗" };
            println!(
                "  {} [{}] {}",
                status, branch.branch_executed, branch.condition
            );
            if let Some(eval) = &branch.evaluated_value {
                println!("    evaluation: {}", eval);
            }
            println!(
                "    lines {}-{}",
                branch.range.start_line + 1,
                branch.range.end_line + 1
            );
        }

        if !result.loop_iterations.is_empty() {
            println!("\n=== Loops ({}) ===", result.loop_iterations.len());
            for loop_info in &result.loop_iterations {
                println!(
                    "  for {} in {} ({} iterations)",
                    loop_info.variable, loop_info.collection, loop_info.iteration_count
                );
                println!(
                    "    lines {}-{}",
                    loop_info.range.start_line + 1,
                    loop_info.range.end_line + 1
                );
            }
        }

        if !result.warnings.is_empty() {
            println!("\n=== Warnings ({}) ===", result.warnings.len());
            for warning in &result.warnings {
                println!("  ⚠ {}", warning);
            }
        }

        return Ok(());
    }

    // Full debug mode
    let provider_config =
        auto_detect_provider().ok_or_else(|| anyhow::anyhow!("No provider configured"))?;

    let analysis_config = AnalysisConfig::from(&provider_config);
    let analysis_client = AnalysisClient::new(analysis_config)?;
    let ir = generate_ir_cached(&analysis_client, &content, &file.display().to_string()).await?;

    let debug_config = DebugConfig {
        context: ctx,
        breakpoints: Vec::new(),
        template_only: false,
        stop_on_entry: false,
    };

    let trace = prompt_runtime::execute(&content, &ir, &provider_config, &debug_config).await?;

    println!("=== Execution Trace ===");
    for step in &trace.steps {
        println!(
            "[{}] {} - {}",
            step.index,
            step.kind,
            step.explanation.as_deref().unwrap_or("")
        );
    }

    if let Some(answer) = &trace.final_answer {
        println!("\n=== Final Answer ===\n{}", answer);
    }

    Ok(())
}

async fn analyze_prompt(
    file: &PathBuf,
    format: &str,
    engine: IrEngine,
    no_lint: bool,
    ablate_ir: bool,
    no_static_lint: bool,
) -> Result<()> {
    let content = std::fs::read_to_string(file)
        .with_context(|| format!("Failed to read file: {}", file.display()))?;
    let uri = file.display().to_string();

    // Build the IR and keep an analysis client (if a provider is available) for linting.
    let (ir, client) = match engine {
        IrEngine::Annotation => {
            info!("Building deterministic IR from annotations...");
            let ir = prompt_template::build_ir(&content, &uri, 1);
            let client = auto_detect_provider()
                .map(|pc| AnalysisClient::new(AnalysisConfig::from(&pc)))
                .transpose()?;
            (ir, client)
        }
        IrEngine::Llm => {
            let provider_config =
                auto_detect_provider().ok_or_else(|| anyhow::anyhow!("No provider configured"))?;
            let analysis_client = AnalysisClient::new(AnalysisConfig::from(&provider_config))?;
            info!("Generating IR...");
            let ir = generate_ir_cached(&analysis_client, &content, &uri).await?;
            (ir, Some(analysis_client))
        }
    };

    // Deterministic static diagnostics (e.g. unreachable rules), provider-free.
    let mut diagnostics = if no_static_lint {
        Vec::new()
    } else {
        prompt_runtime::unreachable_rules(&ir, &content)
    };
    if !no_lint {
        if let Some(client) = &client {
            // For the IR-context ablation, lint against an empty IR so the only
            // difference from the normal run is the presence of structural context.
            let lint_ir = if ablate_ir {
                info!("Running LLM linter WITHOUT IR context (ablation)...");
                prompt_ir::PromptIR::new(uri.clone(), 1)
            } else {
                info!("Running LLM linter...");
                ir.clone()
            };
            diagnostics.extend(client.lint(&content, &lint_ir).await.unwrap_or_default());
        } else {
            info!("No provider configured; static diagnostics only.");
        }
    }

    match format {
        "json" => {
            let output = serde_json::json!({
                "ir": ir,
                "diagnostics": diagnostics,
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
        _ => {
            println!("=== IR Nodes ({}) ===", ir.nodes.len());
            for node in &ir.nodes {
                println!(
                    "  [{:?}] {} - {}",
                    node.kind,
                    node.id,
                    node.label.as_deref().unwrap_or("")
                );
            }

            println!("\n=== Diagnostics ({}) ===", diagnostics.len());
            for diag in &diagnostics {
                println!(
                    "  [{:?}] Line {}: {}",
                    diag.severity, diag.range.start_line, diag.message
                );
            }
        }
    }

    Ok(())
}

fn render_prompt(
    file: &PathBuf,
    context: Option<String>,
    context_file: Option<PathBuf>,
) -> Result<()> {
    let content = std::fs::read_to_string(file)
        .with_context(|| format!("Failed to read file: {}", file.display()))?;

    let ctx = load_context(context, context_file)?;
    let result = prompt_template::render(&content, &ctx)?;

    println!("{}", result.output);

    Ok(())
}

fn show_config() -> Result<()> {
    println!("=== Provider Auto-Detection ===\n");

    let env_vars = [
        ("OPENAI_API_KEY", "OpenAI", "gpt-4"),
        ("ANTHROPIC_API_KEY", "Anthropic", "claude-sonnet-4-20250514"),
        ("OLLAMA_API_KEY", "OllamaCloud", "gpt-oss:120b"),
    ];

    for (env_var, provider, model) in env_vars {
        let status = if std::env::var(env_var).is_ok() {
            "✓ Set"
        } else {
            "✗ Not set"
        };
        println!("  {} {} → {} ({})", status, env_var, provider, model);
    }

    println!();

    if let Some(config) = auto_detect_provider() {
        println!("Active provider: {} ({})", config.provider, config.model);
    } else {
        println!("No provider auto-detected. Set an API key environment variable.");
    }

    Ok(())
}

#[derive(serde::Serialize)]
struct BenchRow {
    file: String,
    total_rules: usize,
    rule_coverage_structural: f64,
    rule_coverage_trace: f64,
    condition_coverage: f64,
    branch_coverage: f64,
    total_rule_pairs: usize,
    rule_pair_coverage: f64,
    mutation_score: Option<f64>,
    total_mutants: Option<usize>,
    killed_mutants: Option<usize>,
}

/// Recursively collect `.prompt.rtpl` files under `dir`.
fn collect_prompts(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir)
        .with_context(|| format!("Failed to read directory: {}", dir.display()))?
    {
        let path = entry?.path();
        if path.is_dir() {
            collect_prompts(&path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("rtpl") {
            out.push(path);
        }
    }
    Ok(())
}

/// The base name of a prompt file, with `.prompt.rtpl`/`.rtpl` stripped.
fn prompt_stem(file: &Path) -> String {
    let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name.strip_suffix(".prompt.rtpl")
        .or_else(|| name.strip_suffix(".rtpl"))
        .unwrap_or(name)
        .to_string()
}

async fn bench_cmd(
    dir: &Path,
    contexts_path: Option<&Path>,
    contexts_dir: Option<&Path>,
    mutation: bool,
    format: &str,
) -> Result<()> {
    let mut files = Vec::new();
    collect_prompts(dir, &mut files)?;
    files.sort();
    let shared_contexts = load_contexts(contexts_path)?;

    let mut rows = Vec::with_capacity(files.len());
    let mut skipped = 0usize;
    for file in &files {
        let uri = file.display().to_string();
        let content = match std::fs::read_to_string(file) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("skip {uri}: {e}");
                skipped += 1;
                continue;
            }
        };
        // Prefer per-prompt contexts when a contexts dir is given.
        let per_prompt = contexts_dir
            .map(|d| d.join(format!("{}.contexts.json", prompt_stem(file))))
            .filter(|p| p.exists());
        let contexts = match &per_prompt {
            Some(p) => load_contexts(Some(p.as_path()))?,
            None => shared_contexts.clone(),
        };
        let ir = prompt_template::build_ir(&content, &uri, 1);
        // A single prompt that fails to render (e.g. a loop over a variable the shared
        // contexts don't define) must not abort the whole corpus run.
        let cov = match prompt_runtime::compute_coverage(&ir, &content, &contexts).await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("skip {uri}: {e}");
                skipped += 1;
                continue;
            }
        };
        let (mutation_score, total_mutants, killed_mutants) = if mutation {
            match prompt_runtime::run_mutation_testing_default(&ir, &content, &contexts) {
                Ok(m) => (
                    Some(m.mutation_score),
                    Some(m.total_mutants),
                    Some(m.killed),
                ),
                Err(_) => (None, None, None),
            }
        } else {
            (None, None, None)
        };
        rows.push(BenchRow {
            file: uri,
            total_rules: cov.total_rules,
            rule_coverage_structural: cov.rule_coverage_structural,
            rule_coverage_trace: cov.rule_coverage_trace,
            condition_coverage: cov.condition_coverage,
            branch_coverage: cov.branch_coverage,
            total_rule_pairs: cov.total_rule_pairs,
            rule_pair_coverage: cov.rule_pair_coverage,
            mutation_score,
            total_mutants,
            killed_mutants,
        });
    }
    if skipped > 0 {
        eprintln!(
            "bench: {} prompt(s) measured, {} skipped",
            rows.len(),
            skipped
        );
    }

    match format {
        "json" => println!("{}", serde_json::to_string_pretty(&rows)?),
        _ => {
            println!("file,total_rules,rule_cov_structural,rule_cov_trace,condition_cov,branch_cov,total_rule_pairs,rule_pair_cov,mutation_score,total_mutants,killed_mutants");
            let opt_f = |v: Option<f64>| v.map(|x| format!("{x:.4}")).unwrap_or_default();
            let opt_u = |v: Option<usize>| v.map(|x| x.to_string()).unwrap_or_default();
            for r in &rows {
                println!(
                    "{},{},{:.4},{:.4},{:.4},{:.4},{},{:.4},{},{},{}",
                    r.file,
                    r.total_rules,
                    r.rule_coverage_structural,
                    r.rule_coverage_trace,
                    r.condition_coverage,
                    r.branch_coverage,
                    r.total_rule_pairs,
                    r.rule_pair_coverage,
                    opt_f(r.mutation_score),
                    opt_u(r.total_mutants),
                    opt_u(r.killed_mutants),
                );
            }
        }
    }
    Ok(())
}

/// Generate IR via the analysis provider, served from the content-addressed IR
/// cache when available. With `--cache-dir` this gives cross-invocation replay.
async fn generate_ir_cached(
    client: &AnalysisClient,
    content: &str,
    uri: &str,
) -> Result<prompt_ir::PromptIR> {
    if let Some(ir) = prompt_runtime::global_cache().get_ir(content) {
        return Ok(ir);
    }
    let ir = client.generate_ir(content, uri, 1).await?;
    prompt_runtime::global_cache().put_ir(content, ir.clone());
    Ok(ir)
}

fn gen_contexts_cmd(file: &Path, out: Option<&Path>) -> Result<()> {
    let content = std::fs::read_to_string(file)
        .with_context(|| format!("Failed to read file: {}", file.display()))?;
    let contexts = prompt_template::generate_contexts(&content);
    let json = serde_json::to_string_pretty(&contexts)?;
    match out {
        Some(path) => {
            std::fs::write(path, &json)
                .with_context(|| format!("Failed to write {}", path.display()))?;
            info!("Wrote {} contexts to {}", contexts.len(), path.display());
        }
        None => println!("{json}"),
    }
    Ok(())
}

async fn coverage_cmd(
    file: &Path,
    contexts_path: Option<&Path>,
    engine: IrEngine,
    format: &str,
) -> Result<()> {
    let content = std::fs::read_to_string(file)
        .with_context(|| format!("Failed to read file: {}", file.display()))?;
    let uri = file.display().to_string();

    let ir = match engine {
        IrEngine::Annotation => prompt_template::build_ir(&content, &uri, 1),
        IrEngine::Llm => {
            let provider_config = auto_detect_provider().ok_or_else(|| {
                anyhow::anyhow!("No provider configured (required for --engine llm)")
            })?;
            let analysis_client = AnalysisClient::new(AnalysisConfig::from(&provider_config))?;
            generate_ir_cached(&analysis_client, &content, &uri).await?
        }
    };

    let contexts = load_contexts(contexts_path)?;
    let report = prompt_runtime::compute_coverage(&ir, &content, &contexts).await?;

    match format {
        "json" => println!("{}", serde_json::to_string_pretty(&report)?),
        _ => print_coverage_pretty(&report),
    }
    Ok(())
}

/// Load input contexts from a JSON file. Accepts an array of objects or a single
/// object. Returns an empty vec when no path is given (coverage then uses `{}`).
fn load_contexts(path: Option<&Path>) -> Result<Vec<Value>> {
    let Some(path) = path else {
        return Ok(Vec::new());
    };
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read contexts file: {}", path.display()))?;
    let value: Value = serde_json::from_str(&raw)
        .with_context(|| format!("Failed to parse contexts JSON: {}", path.display()))?;
    match value {
        Value::Array(arr) => Ok(arr),
        other => Ok(vec![other]),
    }
}

fn print_coverage_pretty(r: &prompt_runtime::CoverageReport) {
    let pct = |x: f64| x * 100.0;
    println!(
        "=== Coverage: {} ({} input{}) ===\n",
        r.uri,
        r.num_inputs,
        if r.num_inputs == 1 { "" } else { "s" }
    );
    println!(
        "Rule coverage (structural): {}/{} ({:.0}%)",
        r.covered_rules_structural,
        r.total_rules,
        pct(r.rule_coverage_structural)
    );
    println!(
        "Rule coverage (trace):      {}/{} ({:.0}%)",
        r.covered_rules_trace,
        r.total_rules,
        pct(r.rule_coverage_trace)
    );
    if !r.uncovered_rules_structural.is_empty() {
        println!(
            "  Uncovered rules: {}",
            r.uncovered_rules_structural.join(", ")
        );
    }
    println!(
        "Condition coverage:         {}/{} ({:.0}%)",
        r.covered_conditions,
        r.total_conditions,
        pct(r.condition_coverage)
    );
    println!(
        "Branch outcome coverage:    {}/{} ({:.0}%)",
        r.covered_branch_outcomes,
        r.total_branch_outcomes,
        pct(r.branch_coverage)
    );
    for b in &r.branches {
        let mark = |seen: bool| if seen { "✓" } else { "✗" };
        println!(
            "  L{} if {} → true {} | false {}",
            b.condition_line + 1,
            b.condition,
            mark(b.true_seen),
            mark(b.false_seen)
        );
    }
    println!(
        "Rule-pair coverage:         {}/{} ({:.0}%)",
        r.covered_rule_pairs,
        r.total_rule_pairs,
        pct(r.rule_pair_coverage)
    );
}

async fn mutate_cmd(
    file: &Path,
    contexts_path: Option<&Path>,
    engine: IrEngine,
    list: bool,
    oracle: OracleKind,
    format: &str,
) -> Result<()> {
    let content = std::fs::read_to_string(file)
        .with_context(|| format!("Failed to read file: {}", file.display()))?;
    let uri = file.display().to_string();

    if list {
        let mutants = prompt_runtime::generate_mutants(&content);
        match format {
            "json" => println!("{}", serde_json::to_string_pretty(&mutants)?),
            _ => {
                println!("=== {} mutants ===", mutants.len());
                for m in &mutants {
                    println!("  {:<22} {}", m.id, m.description);
                }
            }
        }
        return Ok(());
    }

    let ir = match engine {
        IrEngine::Annotation => prompt_template::build_ir(&content, &uri, 1),
        IrEngine::Llm => {
            let provider_config = auto_detect_provider().ok_or_else(|| {
                anyhow::anyhow!("No provider configured (required for --engine llm)")
            })?;
            let analysis_client = AnalysisClient::new(AnalysisConfig::from(&provider_config))?;
            generate_ir_cached(&analysis_client, &content, &uri).await?
        }
    };

    let contexts = load_contexts(contexts_path)?;
    let report = match oracle {
        OracleKind::Trace => prompt_runtime::run_mutation_testing(
            &ir,
            &content,
            &contexts,
            &prompt_runtime::TraceOracle,
        )?,
        OracleKind::Output => prompt_runtime::run_mutation_testing(
            &ir,
            &content,
            &contexts,
            &prompt_runtime::OutputOracle,
        )?,
    };

    match format {
        "json" => println!("{}", serde_json::to_string_pretty(&report)?),
        _ => print_mutation_pretty(&report),
    }
    Ok(())
}

fn print_mutation_pretty(r: &prompt_runtime::MutationReport) {
    println!(
        "=== Mutation testing: {} ({} input{}) ===\n",
        r.uri,
        r.num_inputs,
        if r.num_inputs == 1 { "" } else { "s" }
    );
    println!(
        "Mutation score: {}/{} killed ({:.0}%)",
        r.killed,
        r.total_mutants,
        r.mutation_score * 100.0
    );
    println!("\nBy operator:");
    for (op, stats) in &r.by_operator {
        // Metadata-only mutants edit annotation comments, which are stripped
        // before the prompt reaches the model, so no output-based oracle can see
        // them. Call the two classes out rather than reporting one total.
        let class = if stats.metadata_only == 0 {
            String::new()
        } else {
            format!(
                "  ({} metadata-only, {} body-carrying)",
                stats.metadata_only,
                stats.total - stats.metadata_only
            )
        };
        println!("  {:<16} {}/{}{}", op, stats.killed, stats.total, class);
    }
    if !r.survivors.is_empty() {
        println!("\nSurvived ({}):", r.survivors.len());
        for s in &r.survivors {
            println!("  {:<22} {}", s.id, s.description);
        }
    }
}

fn load_context(json_str: Option<String>, file: Option<PathBuf>) -> Result<Value> {
    if let Some(path) = file {
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read context file: {}", path.display()))?;
        return serde_json::from_str(&content)
            .with_context(|| "Failed to parse context file as JSON");
    }

    if let Some(json) = json_str {
        return serde_json::from_str(&json).with_context(|| "Failed to parse context as JSON");
    }

    Ok(Value::Object(Default::default()))
}

async fn generate_tests(file: &PathBuf, target_node: Option<&str>, format: &str) -> Result<()> {
    let content = std::fs::read_to_string(file)
        .with_context(|| format!("Failed to read file: {}", file.display()))?;

    let provider_config =
        auto_detect_provider().ok_or_else(|| anyhow::anyhow!("No provider configured"))?;

    info!(
        "Using provider: {} ({})",
        provider_config.provider, provider_config.model
    );

    let analysis_config = AnalysisConfig::from(&provider_config);
    let analysis_client = AnalysisClient::new(analysis_config)?;

    info!("Generating IR...");
    let ir = generate_ir_cached(&analysis_client, &content, &file.display().to_string()).await?;

    // Get target nodes - if specific node requested, use that; otherwise find all rules
    let target_nodes: Vec<&str> = if let Some(node_id) = target_node {
        vec![node_id]
    } else {
        ir.rules().iter().map(|r| r.id.as_str()).collect()
    };

    if target_nodes.is_empty() {
        println!("No rules found in prompt. Add @rule annotations to generate tests.");
        return Ok(());
    }

    info!("Generating tests for {} node(s)...", target_nodes.len());

    let mut all_tests = Vec::new();
    for node_id in &target_nodes {
        match analysis_client.generate_tests(&content, &ir, node_id).await {
            Ok(tests) => {
                all_tests.extend(tests);
            }
            Err(e) => {
                eprintln!("Warning: Failed to generate tests for {}: {}", node_id, e);
            }
        }
    }

    match format {
        "json" => {
            let output = serde_json::json!({
                "tests": all_tests,
                "target_nodes": target_nodes,
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
        _ => {
            println!("=== Generated Test Cases ({}) ===\n", all_tests.len());
            for test in &all_tests {
                use prompt_ir::TestKind;
                let kind_icon = match test.kind {
                    TestKind::Positive => "✓",
                    TestKind::Negative => "✗",
                    TestKind::Edge => "⚠",
                };
                println!("{} [{}] {}", kind_icon, test.kind, test.id);
                println!("  Target: {}", test.target_node_id);
                println!("  Input: \"{}\"", test.input);
                if let Some(expected) = &test.expected_behavior {
                    println!("  Expected: {}", expected);
                }
                if let Some(comment) = &test.comment {
                    println!("  Note: {}", comment);
                }
                println!();
            }
        }
    }

    Ok(())
}

async fn suggest_breakpoints(file: &PathBuf, format: &str) -> Result<()> {
    let content = std::fs::read_to_string(file)
        .with_context(|| format!("Failed to read file: {}", file.display()))?;

    let provider_config =
        auto_detect_provider().ok_or_else(|| anyhow::anyhow!("No provider configured"))?;

    info!(
        "Using provider: {} ({})",
        provider_config.provider, provider_config.model
    );

    let analysis_config = AnalysisConfig::from(&provider_config);
    let analysis_client = AnalysisClient::new(analysis_config)?;

    info!("Generating IR...");
    let ir = generate_ir_cached(&analysis_client, &content, &file.display().to_string()).await?;

    info!("Suggesting breakpoints...");
    let suggestions = analysis_client.suggest_breakpoints(&content, &ir).await?;

    match format {
        "json" => {
            let output = serde_json::json!({
                "suggestions": suggestions,
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
        _ => {
            println!("=== Suggested Breakpoints ({}) ===\n", suggestions.len());

            // Sort by confidence
            let mut sorted = suggestions.clone();
            sorted.sort_by(|a, b| {
                b.confidence
                    .partial_cmp(&a.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            for suggestion in &sorted {
                let confidence_bar = "█".repeat((suggestion.confidence * 5.0) as usize);
                let confidence_empty = "░".repeat(5 - (suggestion.confidence * 5.0) as usize);
                println!(
                    "[{}{}] {} ({:.0}%)",
                    confidence_bar,
                    confidence_empty,
                    suggestion.node_id,
                    suggestion.confidence * 100.0
                );
                println!("  {}\n", suggestion.reason);
            }

            // Show how to add breakpoints
            if !sorted.is_empty() {
                println!("To add a breakpoint, use: // @breakpoint BP:NAME");
                println!("Or set breakpoints by line number in VS Code.");
            }
        }
    }

    Ok(())
}

async fn run_lsp() -> Result<()> {
    prompt_lsp::run_server().await;
    Ok(())
}

async fn run_dap() -> Result<()> {
    prompt_dap::run_server().await;
    Ok(())
}

async fn suggest_fixes(file: &PathBuf, apply: bool, format: &str) -> Result<()> {
    let content = std::fs::read_to_string(file)
        .with_context(|| format!("Failed to read file: {}", file.display()))?;

    let provider_config =
        auto_detect_provider().ok_or_else(|| anyhow::anyhow!("No provider configured"))?;

    info!(
        "Using provider: {} ({})",
        provider_config.provider, provider_config.model
    );

    let analysis_config = AnalysisConfig::from(&provider_config);
    let analysis_client = AnalysisClient::new(analysis_config)?;

    info!("Generating IR...");
    let ir = generate_ir_cached(&analysis_client, &content, &file.display().to_string()).await?;

    info!("Running linter...");
    let diagnostics = analysis_client.lint(&content, &ir).await?;

    if diagnostics.is_empty() {
        println!("No issues found. File is clean.");
        return Ok(());
    }

    info!("Generating fixes for {} issue(s)...", diagnostics.len());
    let fixes = analysis_client
        .suggest_fixes(&content, &diagnostics)
        .await?;

    if fixes.is_empty() {
        println!(
            "No automatic fixes available for the {} issue(s) found.",
            diagnostics.len()
        );
        return Ok(());
    }

    match format {
        "json" => {
            let output = serde_json::json!({
                "diagnostics": diagnostics,
                "fixes": fixes,
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
        _ => {
            println!("=== Diagnostics ({}) ===\n", diagnostics.len());
            for diag in &diagnostics {
                println!(
                    "  [{:?}] Line {}: {}",
                    diag.severity,
                    diag.range.start_line + 1,
                    diag.message
                );
                println!("    Code: {}\n", diag.code.as_deref().unwrap_or("unknown"));
            }

            println!("=== Suggested Fixes ({}) ===\n", fixes.len());
            for fix in &fixes {
                let preferred = if fix.is_preferred { " [preferred]" } else { "" };
                println!("{}{}", fix.title, preferred);
                println!("  Fixes: {}", fix.diagnostic_code);
                println!("  Edits: {} change(s)", fix.edits.len());
                for edit in &fix.edits {
                    let preview = if edit.new_text.len() > 40 {
                        format!("{}...", &edit.new_text[..40])
                    } else {
                        edit.new_text.clone()
                    };
                    println!(
                        "    Line {}:{}-{}:{} → \"{}\"",
                        edit.start_line + 1,
                        edit.start_col,
                        edit.end_line + 1,
                        edit.end_col,
                        preview.replace('\n', "\\n")
                    );
                }
                println!();
            }

            if apply {
                // Apply fixes to the file
                let mut modified_content = content.clone();
                let lines: Vec<&str> = content.lines().collect();

                // Sort edits by position (reverse order to apply from end to start)
                let mut all_edits: Vec<_> = fixes.iter().flat_map(|f| f.edits.iter()).collect();
                all_edits
                    .sort_by(|a, b| (b.start_line, b.start_col).cmp(&(a.start_line, a.start_col)));

                // Apply edits (this is simplified - a robust impl would handle overlaps)
                for edit in all_edits {
                    let start_offset = lines
                        .iter()
                        .take(edit.start_line as usize)
                        .map(|l| l.len() + 1)
                        .sum::<usize>()
                        + edit.start_col as usize;
                    let end_offset = lines
                        .iter()
                        .take(edit.end_line as usize)
                        .map(|l| l.len() + 1)
                        .sum::<usize>()
                        + edit.end_col as usize;

                    if start_offset <= modified_content.len()
                        && end_offset <= modified_content.len()
                    {
                        modified_content.replace_range(start_offset..end_offset, &edit.new_text);
                    }
                }

                std::fs::write(file, &modified_content)
                    .with_context(|| format!("Failed to write file: {}", file.display()))?;

                println!("Applied {} fix(es) to {}", fixes.len(), file.display());
            } else {
                println!("Run with --apply to apply these fixes.");
            }
        }
    }

    Ok(())
}

async fn analyze_skill(file: &PathBuf, format: &str) -> Result<()> {
    let content = std::fs::read_to_string(file)
        .with_context(|| format!("Failed to read file: {}", file.display()))?;

    // Parse skill file
    let skill =
        parse_skill(&content).map_err(|e| anyhow::anyhow!("Failed to parse skill: {}", e))?;

    // Validate frontmatter
    let fm_diagnostics = validate_frontmatter(&skill);

    // Try to get LLM analysis if provider is available
    let (ir, lint_diagnostics) = if let Some(provider_config) = auto_detect_provider() {
        info!(
            "Using provider: {} ({})",
            provider_config.provider, provider_config.model
        );
        let analysis_config = AnalysisConfig::from(&provider_config);
        let analysis_client = AnalysisClient::new(analysis_config)?;

        // Serialize frontmatter for analysis
        let fm_json = serde_json::to_string(&serde_json::json!({
            "name": skill.frontmatter.name,
            "description": skill.frontmatter.description,
            "allowed_tools": skill.frontmatter.allowed_tools,
            "model": skill.frontmatter.model,
            "argument_hint": skill.frontmatter.argument_hint,
            "disable_model_invocation": skill.frontmatter.disable_model_invocation,
        }))?;

        info!("Generating IR...");
        let ir = analysis_client
            .generate_skill_ir(&content, &fm_json, &file.display().to_string(), 1)
            .await?;

        info!("Running linter...");
        let diags = analysis_client.lint_skill(&content, &fm_json, &ir).await?;

        (Some(ir), diags)
    } else {
        info!("No provider configured, performing local analysis only");
        (None, Vec::new())
    };

    match format {
        "json" => {
            let output = serde_json::json!({
                "skill": {
                    "name": skill.frontmatter.name,
                    "description": skill.frontmatter.description,
                    "allowed_tools": skill.frontmatter.allowed_tools,
                    "model": skill.frontmatter.model,
                    "argument_hint": skill.frontmatter.argument_hint,
                },
                "substitutions": {
                    "arguments": skill.arguments_locations.len(),
                    "session_id": skill.session_id_locations.len(),
                },
                "commands": skill.command_executions.len(),
                "links": skill.markdown_links.len(),
                "ir": ir,
                "frontmatter_diagnostics": fm_diagnostics,
                "lint_diagnostics": lint_diagnostics,
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
        _ => {
            println!("=== Skill: {} ===", skill.frontmatter.name);
            println!("  Description: {}", skill.frontmatter.description);
            println!();

            println!("=== Frontmatter Properties ===");
            if !skill.frontmatter.allowed_tools.is_empty() {
                println!(
                    "  allowed-tools: {}",
                    skill.frontmatter.allowed_tools.join(", ")
                );
            }
            if let Some(model) = &skill.frontmatter.model {
                println!("  model: {}", model);
            }
            if let Some(hint) = &skill.frontmatter.argument_hint {
                println!("  argument-hint: {}", hint);
            }
            if skill.frontmatter.disable_model_invocation {
                println!("  disable-model-invocation: true");
            }
            println!();

            println!("=== Substitutions ===");
            println!(
                "  $ARGUMENTS: {} occurrence(s)",
                skill.arguments_locations.len()
            );
            for loc in &skill.arguments_locations {
                println!(
                    "    Line {}:{}",
                    loc.range.start_line + 1,
                    loc.range.start_col + 1
                );
            }
            println!(
                "  ${{CLAUDE_SESSION_ID}}: {} occurrence(s)",
                skill.session_id_locations.len()
            );
            for loc in &skill.session_id_locations {
                println!(
                    "    Line {}:{}",
                    loc.range.start_line + 1,
                    loc.range.start_col + 1
                );
            }
            println!();

            if !skill.command_executions.is_empty() {
                println!(
                    "=== Dynamic Commands ({}) ===",
                    skill.command_executions.len()
                );
                for cmd in &skill.command_executions {
                    println!("  `!{}` at line {}", cmd.command, cmd.range.start_line + 1);
                }
                println!();
            }

            if !skill.markdown_links.is_empty() {
                println!("=== File References ({}) ===", skill.markdown_links.len());
                for link in &skill.markdown_links {
                    println!(
                        "  [{}]({}) at line {}",
                        link.text,
                        link.target,
                        link.range.start_line + 1
                    );
                }
                println!();
            }

            if let Some(ir) = &ir {
                println!("=== IR Nodes ({}) ===", ir.nodes.len());
                for node in &ir.nodes {
                    println!(
                        "  [{:?}] {} - {}",
                        node.kind,
                        node.id,
                        node.label.as_deref().unwrap_or("")
                    );
                }
                println!();
            }

            let total_diags = fm_diagnostics.len() + lint_diagnostics.len();
            if total_diags > 0 {
                println!("=== Diagnostics ({}) ===", total_diags);

                for diag in &fm_diagnostics {
                    let icon = match diag.severity {
                        prompt_template::skill::DiagnosticSeverity::Error => "✗",
                        prompt_template::skill::DiagnosticSeverity::Warning => "⚠",
                        prompt_template::skill::DiagnosticSeverity::Info => "ℹ",
                    };
                    println!(
                        "  {} [{}] Line {}: {}",
                        icon,
                        diag.code,
                        diag.range.start_line + 1,
                        diag.message
                    );
                }

                for diag in &lint_diagnostics {
                    let icon = match diag.severity {
                        prompt_ir::DiagnosticSeverity::Error => "✗",
                        prompt_ir::DiagnosticSeverity::Warning => "⚠",
                        prompt_ir::DiagnosticSeverity::Info => "ℹ",
                    };
                    println!(
                        "  {} Line {}: {}",
                        icon,
                        diag.range.start_line + 1,
                        diag.message
                    );
                }
            } else {
                println!("No issues found.");
            }
        }
    }

    Ok(())
}

fn validate_skill(dir: &Path, format: &str) -> Result<()> {
    // Load and validate skill directory
    let skill_dir = SkillDirectory::load(dir)
        .map_err(|e| anyhow::anyhow!("Failed to load skill directory: {}", e))?;

    // Run validation
    let diagnostics = skill_dir.validate();

    // Count files
    let skill_lines = skill_dir.skill_file.body.lines().count();

    match format {
        "json" => {
            let output = serde_json::json!({
                "root": skill_dir.root,
                "skill": {
                    "name": skill_dir.skill_file.frontmatter.name,
                    "description": skill_dir.skill_file.frontmatter.description,
                    "body_lines": skill_lines,
                },
                "references": skill_dir.references.iter().map(|r| serde_json::json!({
                    "path": r.path,
                    "line_count": r.line_count,
                    "referenced_from": r.referenced_from.iter().map(|l| &l.target).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
                "scripts": skill_dir.scripts.iter().map(|s| serde_json::json!({
                    "path": s.path,
                })).collect::<Vec<_>>(),
                "resources": skill_dir.resources.iter().map(|r| &r.path).collect::<Vec<_>>(),
                "diagnostics": diagnostics,
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
        _ => {
            println!("=== Skill Directory: {} ===", dir.display());
            println!("  SKILL.md: {} lines", skill_lines);
            println!(
                "  references/: {} files ({} total lines)",
                skill_dir.references.len(),
                skill_dir
                    .references
                    .iter()
                    .map(|r| r.line_count)
                    .sum::<usize>()
            );
            println!("  scripts/: {} files", skill_dir.scripts.len());
            println!("  resources/: {} files", skill_dir.resources.len());
            println!();

            if !skill_dir.references.is_empty() {
                println!("=== Reference Files ===");
                for reference in &skill_dir.references {
                    let status = if !reference.referenced_from.is_empty() {
                        format!(
                            "(referenced at line {})",
                            reference
                                .referenced_from
                                .first()
                                .map(|l| l.range.start_line + 1)
                                .unwrap_or(0)
                        )
                    } else {
                        "(not referenced)".to_string()
                    };
                    println!(
                        "  {} {} - {} lines",
                        reference.path.display(),
                        status,
                        reference.line_count
                    );
                }
                println!();
            }

            if !skill_dir.scripts.is_empty() {
                println!("=== Script Files ===");
                for script in &skill_dir.scripts {
                    println!("  {}", script.path.display());
                }
                println!();
            }

            if diagnostics.is_empty() {
                println!("No issues found.");
            } else {
                println!("=== Diagnostics ({}) ===", diagnostics.len());
                for diag in &diagnostics {
                    let icon = match diag.severity {
                        prompt_template::skill::DiagnosticSeverity::Error => "✗",
                        prompt_template::skill::DiagnosticSeverity::Warning => "⚠",
                        prompt_template::skill::DiagnosticSeverity::Info => "ℹ",
                    };
                    println!("  {} [{}] {}", icon, diag.code, diag.message);
                    if let Some(path) = &diag.file_path {
                        println!("    at {}", path.display());
                    }
                }
            }

            // Summary
            let errors = diagnostics
                .iter()
                .filter(|d| {
                    matches!(
                        d.severity,
                        prompt_template::skill::DiagnosticSeverity::Error
                    )
                })
                .count();
            let warnings = diagnostics
                .iter()
                .filter(|d| {
                    matches!(
                        d.severity,
                        prompt_template::skill::DiagnosticSeverity::Warning
                    )
                })
                .count();

            println!();
            if errors > 0 || warnings > 0 {
                println!("Summary: {} error(s), {} warning(s)", errors, warnings);
            } else {
                println!("Summary: Skill directory is valid");
            }
        }
    }

    Ok(())
}

async fn run_skill(
    path: &Path,
    args: &str,
    session_id: Option<String>,
    no_commands: bool,
) -> Result<()> {
    // Determine skill directory path
    let skill_path = if path.is_file() {
        path.parent().unwrap_or(path).to_path_buf()
    } else {
        path.to_path_buf()
    };

    // Load skill directory
    let skill_dir = SkillDirectory::load(&skill_path)
        .map_err(|e| anyhow::anyhow!("Failed to load skill: {}", e))?;

    info!("Running skill: {}", skill_dir.skill_file.frontmatter.name);

    // Get provider config (optional for template-only)
    let provider_config = auto_detect_provider();
    if provider_config.is_none() && !no_commands {
        info!("No provider configured - running in template-only mode");
    }

    // Configure execution
    let config = SkillExecutionConfig {
        arguments: args.to_string(),
        session_id,
        working_directory: skill_path.clone(),
        enable_commands: !no_commands,
        command_timeout_ms: 30000,
        template_only: provider_config.is_none(),
        breakpoints: vec![],
    };

    // Execute skill
    let mut executor = SkillExecutor::new(skill_dir, config, provider_config);
    let result = executor
        .execute()
        .await
        .map_err(|e| anyhow::anyhow!("Skill execution failed: {}", e))?;

    // Output result
    if let Some(response) = &result.llm_response {
        println!("{}", response);
    } else {
        // Template-only mode - show the final prompt
        println!("{}", result.final_prompt);
    }

    Ok(())
}

async fn debug_skill(path: &Path, args: &str, template_only: bool, format: &str) -> Result<()> {
    // Determine skill directory path
    let skill_path = if path.is_file() {
        path.parent().unwrap_or(path).to_path_buf()
    } else {
        path.to_path_buf()
    };

    // Load skill directory
    let skill_dir = SkillDirectory::load(&skill_path)
        .map_err(|e| anyhow::anyhow!("Failed to load skill: {}", e))?;

    info!("Debugging skill: {}", skill_dir.skill_file.frontmatter.name);

    // Get provider config
    let provider_config = if template_only {
        None
    } else {
        auto_detect_provider()
    };

    // Configure execution
    let config = SkillExecutionConfig {
        arguments: args.to_string(),
        session_id: None,
        working_directory: skill_path.clone(),
        enable_commands: true,
        command_timeout_ms: 30000,
        template_only: template_only || provider_config.is_none(),
        breakpoints: vec![],
    };

    // Execute skill
    let mut executor = SkillExecutor::new(skill_dir, config, provider_config);
    let result = executor
        .execute()
        .await
        .map_err(|e| anyhow::anyhow!("Skill execution failed: {}", e))?;

    match format {
        "json" => {
            let output = serde_json::json!({
                "trace": result.trace,
                "final_prompt_length": result.final_prompt.len(),
                "commands_executed": result.commands_executed,
                "references_loaded": result.references_loaded,
                "llm_response": result.llm_response,
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
        "trace" => {
            println!("=== Skill Trace: {} ===", result.trace.skill_name);
            println!("Session: {}", result.trace.session_id);
            println!();

            for step in &result.trace.steps {
                let icon = match step.kind {
                    SkillStepKind::Plan => "📋",
                    SkillStepKind::ArgumentSubstitution => "📝",
                    SkillStepKind::SessionIdSubstitution => "🔑",
                    SkillStepKind::ReferenceLoading => "📚",
                    SkillStepKind::CommandExecution => "⚡",
                    SkillStepKind::PromptBuild => "🔧",
                    SkillStepKind::LlmCall => "🤖",
                    SkillStepKind::Summary => "✓",
                };
                println!("[{}] {} {:?}", step.index, icon, step.kind);
                if let Some(explanation) = &step.explanation {
                    println!("    {}", explanation);
                }
            }

            println!("\n=== Status ===");
            println!("Completed: {}", result.trace.completed);
            println!("Commands executed: {}", result.commands_executed.len());
            println!("References loaded: {}", result.references_loaded.len());
            println!("Final prompt: {} chars", result.final_prompt.len());
        }
        _ => {
            // Pretty format
            println!("=== Skill: {} ===", result.trace.skill_name);
            println!("Session: {}", result.trace.session_id);
            println!();

            // Show substitutions
            println!("=== Execution Steps ===");
            for step in &result.trace.steps {
                if let Some(explanation) = &step.explanation {
                    println!("  [{}] {:?}: {}", step.index, step.kind, explanation);
                }
            }
            println!();

            // Show commands executed
            if !result.commands_executed.is_empty() {
                println!(
                    "=== Commands Executed ({}) ===",
                    result.commands_executed.len()
                );
                for cmd in &result.commands_executed {
                    let status = if cmd.exit_code == 0 { "✓" } else { "✗" };
                    println!(
                        "  {} `{}` ({}ms, exit {})",
                        status, cmd.command, cmd.duration_ms, cmd.exit_code
                    );
                    if !cmd.stdout.is_empty() {
                        let preview = if cmd.stdout.len() > 100 {
                            format!("{}...", &cmd.stdout[..100])
                        } else {
                            cmd.stdout.clone()
                        };
                        println!("    stdout: {}", preview.replace('\n', "\\n"));
                    }
                    if !cmd.stderr.is_empty() {
                        println!("    stderr: {}", cmd.stderr.replace('\n', "\\n"));
                    }
                }
                println!();
            }

            // Show references loaded
            if !result.references_loaded.is_empty() {
                println!(
                    "=== References Loaded ({}) ===",
                    result.references_loaded.len()
                );
                for reference in &result.references_loaded {
                    println!(
                        "  [{}] {} ({} lines)",
                        reference.link_text,
                        reference.path.display(),
                        reference.line_count
                    );
                }
                println!();
            }

            // Show final prompt
            println!("=== Final Prompt ({} chars) ===", result.final_prompt.len());
            let preview = if result.final_prompt.len() > 500 {
                format!("{}...\n[truncated]", &result.final_prompt[..500])
            } else {
                result.final_prompt.clone()
            };
            println!("{}", preview);

            // Show LLM response if available
            if let Some(response) = &result.llm_response {
                println!("\n=== LLM Response ===");
                println!("{}", response);
            }
        }
    }

    Ok(())
}

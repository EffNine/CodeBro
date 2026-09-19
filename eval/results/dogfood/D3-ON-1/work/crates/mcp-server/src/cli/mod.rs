#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "codebro")]
#[command(about = "Your AI coding partner in the terminal", long_about = None)]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(long, short, global = true)]
    config: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    /// List the models available from the configured provider.
    ListModels,

    /// Run the engineering runtime as an MCP server over stdio.
    Serve {
        /// Workspace root to serve; defaults to CODEBRO_WORKSPACE_ROOT or the current directory.
        #[arg(long)]
        root: Option<PathBuf>,
        /// Additional workspace root the server may operate on (P8 root
        /// authorization). Repeatable; per-call workspace_root arguments
        /// must resolve to the server root or one of these.
        #[arg(long = "allow-root")]
        allow_root: Vec<PathBuf>,
    },

    /// Scan the workspace and populate .codebro/facts.json.
    Init {
        /// Workspace root to scan; defaults to CODEBRO_WORKSPACE_ROOT or the current directory.
        #[arg(long)]
        root: Option<PathBuf>,
    },

    /// Diagnose the engineering-runtime state of the workspace.
    Doctor {
        /// Workspace root to check; defaults to CODEBRO_WORKSPACE_ROOT or the current directory.
        #[arg(long)]
        root: Option<PathBuf>,
    },

    /// Manage consultant provider authentication.
    Auth {
        #[command(subcommand)]
        command: AuthCommands,
    },

    /// Inspect the engineering fact store.
    Facts {
        #[command(subcommand)]
        command: FactsCommands,
    },

    /// Ask an AI consultant for opinions on architecture, debugging, code review, etc.
    Consult {
        /// Provider to consult: auto or conductor.
        #[arg(long, default_value = "auto")]
        provider: String,
        /// Consultation mode: architecture, debugging, code_review, planning, research, or second_opinion.
        #[arg(long, default_value = "architecture")]
        mode: String,
        /// The question or task to consult on.
        question: String,
        /// Whether to include project facts and engineering memory in the request.
        #[arg(long, default_value_t = false)]
        include_project_context: bool,
        /// Whether to include the current git diff in the request context.
        #[arg(long, default_value_t = false)]
        include_git_diff: bool,
        /// Workspace root; defaults to CODEBRO_WORKSPACE_ROOT or the current directory.
        #[arg(long)]
        root: Option<PathBuf>,
    },

    /// Export portable memory (context records, history, skills) to a directory.
    ///
    /// Deterministic, versioned manifest + JSONL format, secret-redacted,
    /// and byte-stable: the same state exports to the same bytes. Move the
    /// directory to another device and `codebro import` it there.
    Export {
        /// Output directory (created when absent; existing files are
        /// replaced atomically).
        #[arg(long)]
        out: PathBuf,
        /// Print the machine-readable report as JSON.
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// Import a portable memory export into the local CodeBro state.
    ///
    /// Two-phase and explicit: everything is verified and validated before
    /// the first write, authority is preserved verbatim (nothing is ever
    /// promoted to user_confirmed), newer local state is never overwritten,
    /// and skills are inserted without transitions. Re-running converges.
    Import {
        /// Export directory containing manifest.json and *.jsonl.
        #[arg(long = "file", value_name = "DIR")]
        file: PathBuf,
        /// Local workspace root for project-scoped rows; required unless
        /// every source root is given an explicit --map.
        #[arg(long)]
        root: Option<PathBuf>,
        /// Explicit source-root=local-root remapping (repeatable).
        #[arg(long = "map", value_name = "OLD=NEW")]
        map: Vec<String>,
        /// Validate and report without writing anything.
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        /// Do not publish SKILL.md artifacts for imported active skills.
        #[arg(long = "no-publish", default_value_t = false)]
        no_publish: bool,
        /// Provenance label recorded on imported records (default:
        /// `import:<directory>`).
        #[arg(long = "origin")]
        origin: Option<String>,
        /// Print the machine-readable report as JSON.
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// Print the bounded engineering context packet (host/hook integration surface).
    ///
    /// Read-only: identical retrieval semantics, budgets, and provenance to
    /// the `context` MCP tool. Omit --task for the structural session-start
    /// digest. Emits one JSON document on stdout so host hooks (e.g. an
    /// OpenCode plugin) can inject context without an MCP client.
    Context {
        /// Workspace root; defaults to CODEBRO_WORKSPACE_ROOT or the current directory.
        #[arg(long)]
        root: Option<PathBuf>,
        /// The task in the agent's own words; omit for a structural digest.
        #[arg(long)]
        task: Option<String>,
        /// Extra keyword hints for fact/memory/record retrieval (repeatable).
        #[arg(long = "keyword", value_name = "TEXT")]
        keywords: Vec<String>,
        /// Task identity for task-scoped context resolution (task > project > global).
        #[arg(long = "task-id")]
        task_id: Option<String>,
        /// Pretty-print the JSON payload (default: single-line, hook-friendly).
        #[arg(long, default_value_t = false)]
        pretty: bool,
    },
}

#[derive(Subcommand)]
enum AuthCommands {
    /// Show authentication status for all consultant providers.
    Status,
}

#[derive(Subcommand)]
enum FactsCommands {
    /// Diff the current repository state against the last indexed state and
    /// project the engineering impact (modules, symbols, tests).
    Diff {
        /// Workspace root; defaults to CODEBRO_WORKSPACE_ROOT or the current directory.
        #[arg(long)]
        root: Option<PathBuf>,
    },
}

pub async fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        None => {
            use clap::CommandFactory;
            Cli::command().print_help()?;
            println!();
        }
        Some(Commands::ListModels) => {
            let config = crate::config::Config::load()?;
            let api_key = config
                .api_key
                .clone()
                .or_else(|| std::env::var("CODEBRO_API_KEY").ok());

            let base_url = config.base_url.clone();
            let key = api_key.clone();
            let models = std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                rt.block_on(crate::providers::fetch_models(&base_url, key.as_deref()))
            })
            .join()
            .map_err(|_| anyhow::anyhow!("model listing thread panicked"))??;

            println!(
                "{} models available from {}:",
                models.len(),
                config.base_url
            );
            for m in models {
                println!("  {}", m);
            }
        }
        Some(Commands::Serve { root, allow_root }) => {
            let (workspace_root, source) = crate::workspace::resolve_with_source(root)?;
            // P8 root authorization: additional roots come only from the
            // operator (--allow-root flags and/or CODEBRO_ALLOW_ROOTS).
            let extra_roots = crate::workspace::resolve_additional_roots(&allow_root);
            tracing::info!(
                "CodeBro serving workspace {} (via {}); {} authorized root(s)",
                workspace_root.display(),
                source,
                extra_roots.len() + 1
            );

            let mcp_result = crate::mcp::serve(workspace_root, extra_roots).await;
            if let Err(e) = &mcp_result {
                tracing::warn!("MCP server exited: {e}");
            }

            return Ok(());
        }
        Some(Commands::Init { root }) => {
            let workspace_root = crate::workspace::resolve_workspace_root(root)?;
            crate::init::run(&workspace_root)?;
        }
        Some(Commands::Facts { command }) => match command {
            FactsCommands::Diff { root } => {
                let root = crate::workspace::resolve_workspace_root(root)?;
                crate::init::facts_diff(&root)?;
            }
        },
        Some(Commands::Doctor { root }) => {
            let workspace_root = crate::workspace::resolve_workspace_root(root)?;
            let code = crate::doctor::run(&workspace_root)?;
            std::process::exit(code);
        }
        Some(Commands::Auth { command }) => match command {
            AuthCommands::Status => {
                let name = "conductor";
                let status = if name == "conductor" {
                    use crate::consultant::provider::ConsultantProvider as _;
                    let provider =
                        crate::consultant::providers::conductor::ConductorProvider::new();
                    provider.auth_status()
                } else {
                    crate::consultant::types::AuthStatus::Unauthenticated
                };
                println!("{name}: {status}");
            }
        },
        Some(Commands::Consult {
            provider,
            mode,
            question,
            include_project_context,
            include_git_diff,
            root,
        }) => {
            let workspace_root = crate::workspace::resolve_workspace_root(root)?;

            let provider_choice = match provider.as_str() {
                "auto" => crate::consultant::types::ConsultantProvider::Auto,
                "conductor" => crate::consultant::types::ConsultantProvider::Conductor,
                other => {
                    eprintln!("unknown provider '{other}' — use auto or conductor");
                    std::process::exit(1);
                }
            };

            let mode_choice = match mode.as_str() {
                "architecture" => crate::consultant::types::ConsultantMode::Architecture,
                "debugging" => crate::consultant::types::ConsultantMode::Debugging,
                "code_review" | "code-review" => {
                    crate::consultant::types::ConsultantMode::CodeReview
                }
                "planning" => crate::consultant::types::ConsultantMode::Planning,
                "research" => crate::consultant::types::ConsultantMode::Research,
                "second_opinion" | "second-opinion" => {
                    crate::consultant::types::ConsultantMode::SecondOpinion
                }
                other => {
                    eprintln!("unknown mode '{other}' — use architecture, debugging, code_review, planning, research, or second_opinion");
                    std::process::exit(1);
                }
            };

            let mut request = crate::consultant::types::ConsultantRequest {
                provider: provider_choice,
                mode: mode_choice,
                question: question.trim().to_string(),
                context: None,
                files: Vec::new(),
                include_git_diff,
                include_project_context,
                max_answer_length: 0,
            };

            if include_project_context {
                inject_project_context(&mut request, &workspace_root);
            }
            if include_git_diff {
                inject_git_diff(&mut request, &workspace_root);
            }

            let router = crate::consultant::build_router();
            let provider_inst = match router.resolve(&request.provider) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("provider resolution failed: {e}");
                    std::process::exit(1);
                }
            };

            let response = match provider_inst.consult(&request).await {
                Ok(r) => r,
                Err(crate::consultant::provider::ConsultantError::AuthenticationRequired(msg)) => {
                    eprintln!("{msg}");
                    std::process::exit(1);
                }
                Err(e) => {
                    eprintln!("consultation failed: {e}");
                    std::process::exit(1);
                }
            };

            println!("{}", response.answer);
        }
        Some(Commands::Export { out, json }) => {
            let state_dir = crate::mcp::default_state_dir();
            let db_path = state_dir.join(crate::context_runtime::STATE_DB_FILE);
            let report = crate::context_runtime::portability::export_mirror(&db_path, &out)
                .map_err(anyhow::Error::msg)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                let total: usize = report.counts.values().sum();
                println!(
                    "exported {} tables / {} rows to {}",
                    report.counts.len(),
                    total,
                    report.out_dir.display()
                );
                println!("data_hash: {}", report.data_hash);
                println!("redacted string values: {}", report.redacted_values);
            }
        }
        Some(Commands::Import {
            file,
            root,
            map,
            dry_run,
            no_publish,
            origin,
            json,
        }) => {
            use crate::context_runtime::portability::{import_mirror, ImportOptions};
            use std::collections::BTreeMap;
            let workspace = match root {
                Some(path) => Some(crate::workspace::resolve_workspace_root(Some(path))?),
                None => None,
            };
            let mut workspace_map: BTreeMap<String, String> = BTreeMap::new();
            for entry in &map {
                let (old, new) = entry
                    .split_once('=')
                    .ok_or_else(|| anyhow::anyhow!("--map expects OLD=NEW, got {entry:?}"))?;
                if old.trim().is_empty() || new.trim().is_empty() {
                    return Err(anyhow::anyhow!(
                        "--map expects non-empty OLD=NEW, got {entry:?}"
                    ));
                }
                workspace_map.insert(old.trim().to_string(), new.trim().to_string());
            }
            let options = ImportOptions {
                dry_run,
                workspace_map,
                default_workspace: workspace.map(|p| p.display().to_string()),
                import_origin: origin.unwrap_or_else(|| format!("import:{}", file.display())),
                skills_root: if no_publish {
                    None
                } else {
                    crate::mcp::default_skills_root().ok()
                },
            };
            let store = crate::context_packet::open_default_store();
            let report = import_mirror(&store, &file, &options).map_err(anyhow::Error::msg)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "{} — integrity: {}{}",
                    if report.dry_run {
                        "import dry run"
                    } else {
                        "import"
                    },
                    report.integrity,
                    report
                        .format_version
                        .map(|v| format!(" (format v{v})"))
                        .unwrap_or_default()
                );
                for (table, outcome) in &report.tables {
                    println!(
                        "  {table}: inserted {}, updated {}, duplicates {}, conflicts {}, skipped {}",
                        outcome.inserted,
                        outcome.updated,
                        outcome.duplicates,
                        outcome.conflicts,
                        outcome.skipped
                    );
                }
                if !report.not_imported.is_empty() {
                    let names: Vec<&str> = report.not_imported.keys().map(String::as_str).collect();
                    println!("  not imported: {}", names.join(", "));
                }
                for warning in &report.warnings {
                    println!("  warning: {warning}");
                }
            }
        }
        Some(Commands::Context {
            root,
            task,
            keywords,
            task_id,
            pretty,
        }) => {
            let workspace_root = crate::workspace::resolve_workspace_root(root)?;
            let store = crate::context_packet::open_default_store();
            let payload = crate::context_packet::build_context_packet(
                &store,
                &workspace_root,
                task.as_deref().unwrap_or_default(),
                &keywords,
                task_id.as_deref().map(str::trim).filter(|s| !s.is_empty()),
            )
            .map_err(anyhow::Error::msg)?;
            if pretty {
                let value: serde_json::Value =
                    serde_json::from_str(&payload).map_err(anyhow::Error::msg)?;
                println!("{}", serde_json::to_string_pretty(&value)?);
            } else {
                println!("{payload}");
            }
        }
    }

    Ok(())
}

fn inject_project_context(
    request: &mut crate::consultant::types::ConsultantRequest,
    workspace: &std::path::Path,
) {
    let mut ctx_parts: Vec<String> = Vec::new();

    let mut identity = crate::project_identity::ProjectIdentityRuntime::new(workspace);
    if identity.load().is_ok() {
        let snap = identity.snapshot();
        if !snap.name.is_empty() {
            let lang = snap.languages.first().cloned().unwrap_or_default();
            ctx_parts.push(format!("Project: {} ({})", snap.name, lang));
        }
    }

    let store = {
        let path = workspace.join(".codebro/facts.json");
        match std::fs::read(&path) {
            Ok(bytes) => {
                match serde_json::from_slice::<crate::engineering_facts::FactsModel>(&bytes) {
                    Ok(model) => Some(crate::fact_store::FactStore::from_model(&model)),
                    Err(_) => None,
                }
            }
            Err(_) => None,
        }
    };
    if let Some(ref store) = store {
        let counts = store.collection().counts();
        ctx_parts.push(format!(
            "Verified facts: {} symbols, {} modules, {} tests, {} dependencies",
            counts.symbols, counts.modules, counts.tests, counts.dependencies
        ));
    }

    let mut memory = crate::engineering_memory::EngineeringMemoryRuntime::new(
        workspace,
        crate::project_identity::ProjectIdentityRuntime::new(workspace),
    );
    let _ = memory.load();
    let entries = memory.snapshot();
    if !entries.is_empty() {
        let tags: std::collections::BTreeSet<&str> = entries
            .iter()
            .flat_map(|e| e.metadata.tags.iter().map(|s| s.as_str()))
            .collect();
        ctx_parts.push(format!(
            "Engineering memory: {} entries, tags: {}",
            entries.len(),
            tags.iter().copied().take(10).collect::<Vec<_>>().join(", ")
        ));
    }

    if !ctx_parts.is_empty() {
        let context = ctx_parts.join("\n");
        request.context = Some(match request.context.take() {
            Some(existing) => format!("{existing}\n\n--- Project Context ---\n{context}"),
            None => format!("--- Project Context ---\n{context}"),
        });
    }
}

fn inject_git_diff(
    request: &mut crate::consultant::types::ConsultantRequest,
    workspace: &std::path::Path,
) {
    let output = std::process::Command::new("git")
        .current_dir(workspace)
        .args(["diff", "--cached", "--stat"])
        .output();
    let diff = match &output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout).to_string();
            if text.is_empty() {
                let out2 = std::process::Command::new("git")
                    .current_dir(workspace)
                    .args(["diff"])
                    .output();
                match out2 {
                    Ok(o2) if o2.status.success() => {
                        String::from_utf8_lossy(&o2.stdout).to_string()
                    }
                    _ => String::new(),
                }
            } else {
                text
            }
        }
        _ => String::new(),
    };
    if !diff.trim().is_empty() {
        let truncated: String = diff.chars().take(4096).collect();
        let context = format!("--- Git Diff ---\n{truncated}");
        request.context = Some(match request.context.take() {
            Some(existing) => format!("{existing}\n\n{context}"),
            None => context,
        });
    }
}

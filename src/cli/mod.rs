#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
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
        /// Workspace root to serve; defaults to the current directory.
        #[arg(long)]
        root: Option<PathBuf>,
    },

    /// Scan the workspace and populate .codebro/facts.json.
    Init {
        /// Workspace root to scan; defaults to the current directory.
        #[arg(long)]
        root: Option<PathBuf>,
    },

    /// Diagnose the engineering-runtime state of the workspace.
    Doctor {
        /// Workspace root to check; defaults to the current directory.
        #[arg(long)]
        root: Option<PathBuf>,
    },

    /// Manage consultant provider authentication.
    Auth {
        #[command(subcommand)]
        command: AuthCommands,
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
        /// Workspace root; defaults to the current directory.
        #[arg(long)]
        root: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum AuthCommands {
    /// Show authentication status for all consultant providers.
    Status,
}

const FALLBACK_MODEL: &str = "gpt-4o";

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
        Some(Commands::Serve { root }) => {
            let workspace_root = match root {
                Some(p) => p,
                None => std::env::current_dir()?,
            };

            let mcp_result = crate::mcp::serve(workspace_root).await;
            if let Err(e) = &mcp_result {
                tracing::warn!("MCP server exited: {e}");
            }

            return Ok(());
        }
        Some(Commands::Init { root }) => {
            let workspace_root = match root {
                Some(p) => p,
                None => std::env::current_dir()?,
            };
            crate::init::run(&workspace_root)?;
        }
        Some(Commands::Doctor { root }) => {
            let workspace_root = match root {
                Some(p) => p,
                None => std::env::current_dir()?,
            };
            let code = crate::doctor::run(&workspace_root)?;
            std::process::exit(code);
        }
        Some(Commands::Auth { command }) => match command {
            AuthCommands::Status => {
                for name in ["conductor"] {
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
            let workspace_root = match root {
                Some(p) => p,
                None => std::env::current_dir()?,
            };

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
    }

    Ok(())
}

fn inject_project_context(
    request: &mut crate::consultant::types::ConsultantRequest,
    workspace: &std::path::Path,
) {
    let mut ctx_parts: Vec<String> = Vec::new();

    let mut identity = crate::project_identity::ProjectIdentityRuntime::new(workspace);
    if let Ok(_) = identity.load() {
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
            tags.iter()
                .map(|s| *s)
                .take(10)
                .collect::<Vec<_>>()
                .join(", ")
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

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
}

const FALLBACK_MODEL: &str = "gpt-4o";

pub async fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        None => {
            // No subcommand: print help.
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
            crate::mcp::serve(workspace_root).await?;
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
    }

    Ok(())
}

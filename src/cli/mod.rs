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
    Chat,

    /// List the models available from the configured provider.
    ListModels,

    /// Run the interactive onboarding wizard.
    Onboard,

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
}

const FALLBACK_MODEL: &str = "gpt-4o";

/// If no model is configured, query the provider's `/models` endpoint and pick
/// a sensible default. When the endpoint is unavailable or incomplete but the
/// provider has a deterministic official catalog (e.g. DeepSeek), the
/// provider-known fallback is used instead — clearly labelled as such.
/// Persists the choice so future launches are instant.
fn resolve_model(config: &mut crate::config::Config) {
    if config.is_model_set() {
        return;
    }

    let api_key = config
        .api_key
        .clone()
        .or_else(|| std::env::var("CODEBRO_API_KEY").ok());

    // Run discovery on a dedicated thread with its own runtime. We may already
    // be inside the app's tokio runtime (from #[tokio::main]), so we must not
    // block_on on this thread.
    let base_url = config.base_url.clone();
    let key = api_key.clone();
    let provider = config.provider.clone();
    let discovered = std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(_) => return None,
        };
        rt.block_on(crate::providers::discover_models(
            &base_url,
            key.as_deref(),
            &provider,
        ))
        .into()
    })
    .join()
    .ok()
    .flatten();

    match discovered {
        Some(discovery) if !discovery.models.is_empty() => {
            let model = crate::providers::pick_default_from_discovery(&discovery);
            let model = model.unwrap_or_else(|| discovery.models[0].id.clone());
            config.model = model.clone();
            if discovery.used_fallback {
                println!(
                    "Model endpoint unavailable; using provider-known fallback models from {}: {}",
                    config.base_url, model
                );
            } else {
                println!("Auto-detected model: {} (from {})", model, config.base_url);
            }
            let _ = config.persist_model();
        }
        _ => {
            config.model = FALLBACK_MODEL.to_string();
            println!(
                "Could not reach {} to detect a model; defaulting to {}",
                config.base_url, FALLBACK_MODEL
            );
        }
    }
}

pub async fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Chat) | None => {
            let mut config = crate::config::Config::load()?;

            // Check if onboarding is needed
            let config_dir = crate::config::Config::config_dir();
            let onboarding = crate::onboarding::OnboardingManager::new(config_dir.clone());

            if onboarding.check_first_run() {
                // First run - run onboarding wizard
                run_onboarding_wizard(config_dir, &mut config).await?;
            } else {
                resolve_model(&mut config);
            }

            let app = crate::tui::TuiApp::new_with_config(config)?;
            crate::tui::run(app)?;
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
        Some(Commands::Onboard) => {
            let config_dir = crate::config::Config::config_dir();
            run_onboarding_wizard(config_dir, &mut crate::config::Config::load()?).await?;
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
    }

    Ok(())
}

async fn run_onboarding_wizard(
    config_dir: PathBuf,
    config: &mut crate::config::Config,
) -> Result<()> {
    use std::io::{self, Write};

    println!("\n  CodeBro Onboarding Wizard");
    println!("  =========================\n");

    let mut manager = crate::onboarding::OnboardingManager::new(config_dir.clone());
    manager.start();

    // Step 1: API Key
    print!("  Enter your API key (or press Enter to skip if using env var): ");
    io::stdout().flush()?;
    let mut api_key_input = String::new();
    io::stdin().read_line(&mut api_key_input)?;
    let api_key = api_key_input.trim().to_string();

    if !api_key.is_empty() {
        config.api_key = Some(api_key.clone());
        manager.set_api_key(&api_key);
    } else if let Ok(env_key) = std::env::var("CODEBRO_API_KEY") {
        config.api_key = Some(env_key.clone());
        manager.set_api_key(&env_key);
    } else {
        println!("  No API key provided. Set CODEBRO_API_KEY or enter it above.");
        return Err(anyhow::anyhow!("No API key provided"));
    }

    // Step 2: Provider Selection
    println!("\n  Available providers:");
    println!("    1. OpenAI");
    println!("    2. OpenRouter");
    println!("    3. DeepSeek");
    println!("    4. Ollama (local)");
    println!("    5. LM Studio (local)");
    print!("  Select provider (1-5, default: 1): ");
    io::stdout().flush()?;
    let mut provider_input = String::new();
    io::stdin().read_line(&mut provider_input)?;

    let provider = match provider_input.trim() {
        "2" => crate::provider_manager::ProviderId::OpenRouter,
        "3" => crate::provider_manager::ProviderId::DeepSeek,
        "4" => crate::provider_manager::ProviderId::Ollama,
        "5" => crate::provider_manager::ProviderId::LMStudio,
        _ => crate::provider_manager::ProviderId::OpenAI,
    };
    let provider_for_discovery = provider.clone();
    manager.select_provider(&provider);

    // Step 3: Model Detection
    println!("\n  Detecting available models...");
    let model = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();
        match rt {
            Ok(rt) => rt.block_on(crate::providers::discover_model(
                &provider_for_discovery.default_base_url(),
                Some(&api_key),
                provider_for_discovery.as_str(),
            )),
            Err(_) => None,
        }
    })
    .join()
    .ok()
    .flatten();

    let selected_model = match model {
        Some(m) if !m.is_empty() => {
            println!("  Detected model: {}", m);
            m
        }
        _ => {
            println!("  Could not detect model, using default.");
            FALLBACK_MODEL.to_string()
        }
    };
    config.model = selected_model.clone();
    manager.session.wizard_state.selected_model = Some(selected_model);

    // Step 4: Workspace Discovery
    let workspace_root = std::env::current_dir()?;
    println!("\n  Discovering workspace: {}", workspace_root.display());
    manager.discover_workspace(&workspace_root).await;
    manager.discover_capabilities(&workspace_root).await;

    if let Some(ref wd) = manager.session.workspace_discovery {
        println!("\n  Workspace detected:");
        println!("    Language: {}", wd.language);
        if let Some(ref fw) = wd.framework {
            println!("    Framework: {}", fw);
        }
        if let Some(ref bs) = wd.build_system {
            println!("    Build system: {}", bs);
        }
        println!(
            "    Findings: {} integrations available",
            wd.proposals.len()
        );
    }

    // Step 5: Save config
    println!("\n  Saving configuration...");
    config.provider = provider.as_str().to_string();
    config.base_url = provider.default_base_url();
    config.persist_model()?;

    // Mark onboarding as complete
    let settings_path = config_dir.join(".onboarding_complete");
    std::fs::write(&settings_path, "true")?;

    println!("\n  Onboarding complete!");
    println!("  Config saved to: {:?}", config_dir.join("config.toml"));
    println!("\n  Run `codebro` to start the TUI.\n");

    Ok(())
}

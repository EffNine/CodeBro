#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
mod ai_runtime;
mod memory_runtime;
mod adaptive_validation;
mod agent;
mod capability_discovery;
mod cli;
mod config;
mod context;
mod dispatcher;
mod error;
mod indexer;
mod integration_pipeline;
mod intelligence;
mod intent_engine;
mod metrics;
mod observability;
mod onboarding;
mod plugin_sdk;
mod preference_engine;
mod prompt;
mod provider_manager;
mod providers;
mod provider_runtime;
mod recommendation_engine;
mod reliability;
mod runtime;
mod scanner;
mod session;
mod settings;
mod service_registry;
mod tests;
mod tools;
mod tui;
mod workflow_engine;
mod workspace_discovery;

use anyhow::Result;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    info!("CodeBro starting...");
    cli::run().await?;
    info!("CodeBro session ended.");
    Ok(())
}

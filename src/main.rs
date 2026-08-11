#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
mod adaptive_validation;
mod agent;
mod ai_runtime;
mod assembly;
mod cancellation;
mod canonical_runtime;
mod capability_discovery;
mod cli;
mod config;
mod credentials;
mod dispatcher;
mod engineering_context;
mod engineering_facts;
mod engineering_memory;
mod engineering_objective;
mod error;
mod fact_store;
mod integration_pipeline;
mod intelligence;
mod intent_engine;
mod memory_runtime;
mod metrics;
mod observability;
mod onboarding;
mod plugin_sdk;
mod preference_engine;
mod project_identity;
mod prompt_builder;
mod provider_manager;
mod provider_runtime;
mod providers;
mod recommendation_engine;
mod reliability;
mod research;
mod runtime;
mod scanner;
mod service_registry;
mod session;
mod settings;
mod testing;
mod tests;
mod tools;
mod tui;
mod workflow_engine;
mod workspace_discovery;
mod workspace_runtime;

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

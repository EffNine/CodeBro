#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
mod cancellation;
mod cli;
mod coding;
mod config;
mod consultant;
mod credentials;
mod doctor;
mod engineering_facts;
mod engineering_memory;
mod error;
mod fact_store;
mod impact;
mod init;
mod intelligence;
#[cfg(test)]
mod legacy;
mod mcp;
mod memory_runtime;
mod persistence;
mod project_identity;
mod provenance;
mod providers;
mod sandbox;
mod tools;

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

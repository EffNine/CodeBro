#[tokio::main]
async fn main() -> anyhow::Result<()> {
    codebro_mcp_server::run().await
}

//! gitspin CLI binary.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    gitspin_cli::run_cli().await
}

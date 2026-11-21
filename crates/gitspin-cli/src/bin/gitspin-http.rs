//! gitspin HTTP server binary.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    gitspin_cli::run_http().await
}

//! gitspin codegen shim.
//!
//! `cargo run -p` cannot select git-dependency packages, so this thin binary
//! wraps the pinned `hydra-codegen` library and reproduces the upstream CLI
//! contract: `write` regenerates committed artifacts, `check` verifies they
//! are current. Run it from `crates/gitspin-cli` (where `api/operations.yaml`
//! and `hydra.yaml` live).

use anyhow::{Context, Result};
use hydra_codegen::{
    generate_all, validate_generation, verify_generated, write_generated, GenerateConfig,
};
use hydra_core::{DEFAULT_DEFINITION_PATH, DEFAULT_GENERATED_DIR};

fn main() -> Result<()> {
    let command = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "check".to_string());
    let config = load_config()?;
    match command.as_str() {
        "write" => {
            let definition = hydra_core::load_api_definition(DEFAULT_DEFINITION_PATH)?;
            validate_generation(&definition, &config)
                .context("validate generated TypeScript identifiers")?;
            let artifacts = generate_all(&definition, &config);
            write_generated(DEFAULT_GENERATED_DIR, &artifacts)?;
            println!("wrote {DEFAULT_GENERATED_DIR}/cli.rs, http.rs, mcp.json, ts-client/index.ts");
        }
        "check" => {
            verify_generated(DEFAULT_DEFINITION_PATH, DEFAULT_GENERATED_DIR, &config)
                .context("generated artifacts are stale")?;
            println!("generated artifacts are current");
        }
        other => anyhow::bail!("unknown command {other:?}; expected `check` or `write`"),
    }
    Ok(())
}

/// Load optional per-project generation config from `hydra.yaml`.
fn load_config() -> Result<GenerateConfig> {
    let path = std::path::Path::new("hydra.yaml");
    if !path.exists() {
        return Ok(GenerateConfig::default());
    }
    let raw = std::fs::read_to_string(path).context("read hydra.yaml")?;
    let config: GenerateConfig = serde_yaml::from_str(&raw).context("parse hydra.yaml")?;
    config.validate().context("validate hydra.yaml")?;
    Ok(config)
}

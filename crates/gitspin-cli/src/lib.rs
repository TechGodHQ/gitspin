//! gitspin CLI library: the handwritten part of the hydra project.
//!
//! `api/operations.yaml` is the single source of truth; the committed
//! artifacts under `generated/` are produced by `cargo run -p
//! gitspin-codegen --bin gitspin-codegen -- write` and gated by `-- check` in
//! CI. Every generated surface funnels into one dispatch:
//! [`execute_operation`].

use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use clap::Parser;
use gitspin_core::{
    ContributionProvider, LinesChanged, ProviderConfig, ProviderError, RepoRef, TimeWindow,
};
use serde_json::Value;

pub mod generated {
    include!("../generated/http.rs");
}

pub mod generated_cli {
    include!("../generated/cli.rs");
}

/// Constant path to generated MCP tool schemas.
pub const GENERATED_MCP_JSON: &str = include_str!("../generated/mcp.json");

/// Shared application state: the active provider adapter.
#[derive(Clone)]
pub struct AppState {
    provider: Arc<dyn ContributionProvider>,
}

impl AppState {
    /// Build state from a provider configuration.
    ///
    /// # Errors
    /// When the adapter cannot be constructed (e.g. invalid HTTP client).
    pub fn from_config(config: &ProviderConfig) -> Result<Self, ProviderError> {
        let provider: Arc<dyn ContributionProvider> = match config {
            ProviderConfig::GitHub { token } => {
                Arc::new(gitspin_github::GitHubProvider::new(token.clone())?)
            }
            other => {
                return Err(ProviderError::NotImplemented(other.name().to_string()));
            }
        };
        Ok(Self { provider })
    }

    /// Build state from an arbitrary provider (tests, embedding).
    #[must_use]
    pub fn with_provider(provider: Arc<dyn ContributionProvider>) -> Self {
        Self { provider }
    }

    /// Active provider id.
    #[must_use]
    pub fn provider_name(&self) -> &'static str {
        self.provider.name()
    }
}

/// Typed operation error carrying an HTTP status, so every surface reports
/// the same status/message pair.
#[derive(Debug)]
pub struct OperationError {
    /// HTTP status for the HTTP surface.
    pub status: StatusCode,
    /// Human-readable message used on all surfaces.
    pub message: String,
}

impl std::fmt::Display for OperationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for OperationError {}

fn bad_request(message: String) -> OperationError {
    OperationError {
        status: StatusCode::BAD_REQUEST,
        message,
    }
}

fn provider_error(err: ProviderError) -> OperationError {
    let status = match &err {
        ProviderError::NotImplemented(_) => StatusCode::NOT_IMPLEMENTED,
        _ => StatusCode::BAD_GATEWAY,
    };
    OperationError {
        status,
        message: err.to_string(),
    }
}

fn query_str(
    input: &generated::GeneratedOperationInput,
    key: &str,
) -> Result<String, OperationError> {
    input
        .query
        .get(key)
        .cloned()
        .ok_or_else(|| bad_request(format!("missing query parameter: {key}")))
}

fn query_opt(input: &generated::GeneratedOperationInput, key: &str) -> Option<String> {
    input.query.get(key).cloned()
}

/// The single dispatch every generated surface funnels through.
///
/// # Errors
/// Returns a typed [`OperationError`] for unknown operations, missing
/// parameters, unsupported providers, and provider failures.
pub async fn execute_operation(
    state: &AppState,
    operation: &str,
    input: generated::GeneratedOperationInput,
) -> Result<Value, OperationError> {
    let provider_param = input
        .query
        .get("provider")
        .cloned()
        .unwrap_or_else(|| state.provider_name().to_string());
    if provider_param != state.provider_name() {
        return Err(OperationError {
            status: StatusCode::NOT_IMPLEMENTED,
            message: format!(
                "provider '{provider_param}' is not available (active: {})",
                state.provider_name()
            ),
        });
    }
    if operation == "provider_info" {
        return Ok(serde_json::json!({
            "provider": state.provider_name(),
            "operations": ["list_contributors", "list_commits", "lines_changed", "list_reviews"],
        }));
    }
    let repo = RepoRef::new(query_str(&input, "owner")?, query_str(&input, "repo")?);
    let window = TimeWindow {
        since: query_opt(&input, "since"),
        until: query_opt(&input, "until"),
    };

    match operation {
        "list_contributors" => {
            let repos = vec![repo];
            let contributors = state
                .provider
                .contributors(&repos, &window)
                .await
                .map_err(provider_error)?;
            serde_json::to_value(contributors)
                .map_err(|e| bad_request(format!("serialize failed: {e}")))
        }
        "list_commits" => {
            let repos = vec![repo];
            let commits = state
                .provider
                .commits(&repos, &window)
                .await
                .map_err(provider_error)?;
            serde_json::to_value(commits).map_err(|e| bad_request(format!("serialize failed: {e}")))
        }
        "lines_changed" => {
            let repos = vec![repo];
            let lines: LinesChanged = state
                .provider
                .lines_changed(&repos, &window)
                .await
                .map_err(provider_error)?;
            serde_json::to_value(lines).map_err(|e| bad_request(format!("serialize failed: {e}")))
        }
        "list_reviews" => {
            let repos = vec![repo];
            let reviews = state
                .provider
                .reviews(&repos, &window)
                .await
                .map_err(provider_error)?;
            serde_json::to_value(reviews).map_err(|e| bad_request(format!("serialize failed: {e}")))
        }
        other => Err(bad_request(format!("unknown operation: {other}"))),
    }
}

/// HTTP adapter: run the operation and map typed errors to status codes.
///
/// # Panics
/// Never.
#[allow(clippy::unused_async)]
pub async fn execute_operation_http(
    state: &AppState,
    operation: &str,
    input: generated::GeneratedOperationInput,
) -> Response {
    match execute_operation(state, operation, input).await {
        Ok(value) => axum::Json(value).into_response(),
        Err(err) => (
            err.status,
            axum::Json(serde_json::json!({ "error": err.message })),
        )
            .into_response(),
    }
}

/// Build the HTTP router for gitspin.
pub fn http_router(state: AppState) -> axum::Router {
    generated::generated_router().with_state(state)
}

/// HTTP entry: serve the generated routes on localhost.
///
/// # Errors
/// Propagates listener/serve failures.
pub async fn run_http() -> anyhow::Result<()> {
    let state = AppState::from_config(&ProviderConfig::default())
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let app = http_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8943").await?;
    println!("listening on http://127.0.0.1:8943");
    axum::serve(listener, app).await?;
    Ok(())
}

/// CLI entry: parse args, dispatch through the same operation path.
///
/// # Errors
/// Propagates dispatch failures.
pub async fn run_cli() -> anyhow::Result<()> {
    #[derive(Parser)]
    #[command(name = "gitspin", about = "Contribution tracking across git forges")]
    struct Cli {
        #[command(subcommand)]
        command: generated_cli::GeneratedCommand,
    }

    let cli = Cli::parse();
    let state = AppState::from_config(&ProviderConfig::default())
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    let operation = cli.command.operation_name();
    let params = cli.command.parameters_json();
    let string_params: std::collections::BTreeMap<String, String> = params
        .as_object()
        .map(|o| {
            o.iter()
                .filter(|(_, v)| v.is_string())
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
                .collect()
        })
        .unwrap_or_default();
    let input = generated::GeneratedOperationInput {
        path: std::collections::BTreeMap::new(),
        query: string_params.clone(),
        body: Value::Null,
    };

    let value = execute_operation(&state, operation, input)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitspin_core::{CommitSummary, ContributorSummary, ReviewSummary};
    use std::collections::BTreeMap;

    /// In-memory provider for dispatch tests.
    struct FixtureProvider;

    #[async_trait::async_trait]
    impl ContributionProvider for FixtureProvider {
        fn name(&self) -> &'static str {
            "github"
        }
        async fn contributors(
            &self,
            _repos: &[RepoRef],
            _window: &TimeWindow,
        ) -> Result<Vec<ContributorSummary>, ProviderError> {
            Ok(vec![ContributorSummary {
                login: "shiv".into(),
                commit_count: 3,
            }])
        }
        async fn commits(
            &self,
            _repos: &[RepoRef],
            _window: &TimeWindow,
        ) -> Result<Vec<CommitSummary>, ProviderError> {
            Ok(vec![CommitSummary {
                id: "abc".into(),
                author: Some("shiv".into()),
                message: "feat: bootstrap".into(),
                additions: 10,
                deletions: 2,
            }])
        }
        async fn lines_changed(
            &self,
            _repos: &[RepoRef],
            _window: &TimeWindow,
        ) -> Result<LinesChanged, ProviderError> {
            Ok(LinesChanged {
                added: 10,
                removed: 2,
            })
        }
        async fn reviews(
            &self,
            _repos: &[RepoRef],
            _window: &TimeWindow,
        ) -> Result<Vec<ReviewSummary>, ProviderError> {
            Ok(vec![ReviewSummary {
                pr_number: 1,
                reviewer: "alice".into(),
                state: "approved".into(),
                submitted_at: None,
            }])
        }
    }

    fn input(pairs: &[(&str, &str)]) -> generated::GeneratedOperationInput {
        generated::GeneratedOperationInput {
            path: BTreeMap::new(),
            query: pairs
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            body: Value::Null,
        }
    }

    #[tokio::test]
    async fn provider_info_reports_adapter() {
        let state = AppState::with_provider(Arc::new(FixtureProvider));
        let value = execute_operation(&state, "provider_info", input(&[("provider", "github")]))
            .await
            .expect("dispatch");
        assert_eq!(value["provider"], "github");
    }

    #[tokio::test]
    async fn list_contributors_dispatches() {
        let state = AppState::with_provider(Arc::new(FixtureProvider));
        let value = execute_operation(
            &state,
            "list_contributors",
            input(&[
                ("provider", "github"),
                ("owner", "TechGodHQ"),
                ("repo", "gitspin"),
            ]),
        )
        .await
        .expect("dispatch");
        assert_eq!(value[0]["login"], "shiv");
        assert_eq!(value[0]["commit_count"], 3);
    }

    #[tokio::test]
    async fn lines_changed_dispatches() {
        let state = AppState::with_provider(Arc::new(FixtureProvider));
        let value = execute_operation(
            &state,
            "lines_changed",
            input(&[("provider", "github"), ("owner", "o"), ("repo", "r")]),
        )
        .await
        .expect("dispatch");
        assert_eq!(value["added"], 10);
        assert_eq!(value["removed"], 2);
    }

    #[tokio::test]
    async fn list_reviews_dispatches() {
        let state = AppState::with_provider(Arc::new(FixtureProvider));
        let value = execute_operation(
            &state,
            "list_reviews",
            input(&[("provider", "github"), ("owner", "o"), ("repo", "r")]),
        )
        .await
        .expect("dispatch");
        assert_eq!(value[0]["reviewer"], "alice");
    }

    #[tokio::test]
    async fn unknown_provider_is_rejected() {
        let state = AppState::with_provider(Arc::new(FixtureProvider));
        let err = execute_operation(
            &state,
            "list_commits",
            input(&[("provider", "gitlab"), ("owner", "o"), ("repo", "r")]),
        )
        .await
        .expect_err("should reject");
        assert_eq!(err.status, StatusCode::NOT_IMPLEMENTED);
    }

    #[tokio::test]
    async fn missing_params_are_bad_request() {
        let state = AppState::with_provider(Arc::new(FixtureProvider));
        let err = execute_operation(&state, "list_commits", input(&[("provider", "github")]))
            .await
            .expect_err("should reject");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn unknown_operation_is_rejected() {
        let state = AppState::with_provider(Arc::new(FixtureProvider));
        let err = execute_operation(&state, "nope", input(&[]))
            .await
            .expect_err("should reject");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn generated_mcp_manifest_is_embedded() {
        assert!(GENERATED_MCP_JSON.contains("list_contributors"));
    }

    #[test]
    fn cli_config_yaml_matches_provider_config() {
        let yaml = "provider:\n  type: github\n  token: null\nrepos: []\n";
        let cfg: gitspin_core::GitspinConfig = serde_yaml::from_str(yaml).expect("parse");
        assert_eq!(cfg.provider.name(), "github");
    }
}

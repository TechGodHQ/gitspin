//! The provider trait every forge adapter must implement.

use crate::types::{
    CommitSummary, ContributorSummary, LinesChanged, RepoRef, ReviewSummary, TimeWindow,
};
use async_trait::async_trait;

/// Errors surfaced by a provider adapter.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// The provider is declared but not yet implemented.
    #[error("provider '{0}' is not implemented yet")]
    NotImplemented(String),
    /// Transport or API failure.
    #[error("provider request failed: {0}")]
    Request(String),
    /// The provider response could not be interpreted.
    #[error("unexpected provider response: {0}")]
    Response(String),
}

/// A read-only adapter over one git forge.
///
/// Implementations aggregate over the given repos and time window. All
/// methods are async so real transports can await I/O; aggregation across
/// providers happens one level up.
#[async_trait]
pub trait ContributionProvider: Send + Sync {
    /// Stable provider identifier (e.g. "github").
    fn name(&self) -> &'static str;

    /// Per-contributor commit counts.
    async fn contributors(
        &self,
        repos: &[RepoRef],
        window: &TimeWindow,
    ) -> Result<Vec<ContributorSummary>, ProviderError>;

    /// Individual commits across the repos.
    async fn commits(
        &self,
        repos: &[RepoRef],
        window: &TimeWindow,
    ) -> Result<Vec<CommitSummary>, ProviderError>;

    /// Aggregate lines added/removed.
    async fn lines_changed(
        &self,
        repos: &[RepoRef],
        window: &TimeWindow,
    ) -> Result<LinesChanged, ProviderError>;

    /// PR reviews across the repos.
    async fn reviews(
        &self,
        repos: &[RepoRef],
        window: &TimeWindow,
    ) -> Result<Vec<ReviewSummary>, ProviderError>;
}

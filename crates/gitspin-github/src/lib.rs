//! GitHub adapter: read-only REST implementation of `ContributionProvider`.
//!
//! Minimal by design — three read endpoints (commits, contributors, PR
//! reviews) plus pure response parsers that are unit-tested without network.

use std::time::Duration;

use async_trait::async_trait;
use gitspin_core::{
    CommitSummary, ContributionProvider, ContributorSummary, LinesChanged, ProviderError, RepoRef,
    ReviewSummary, TimeWindow,
};
use serde_json::Value;

/// Default GitHub REST API base.
pub const DEFAULT_API_BASE: &str = "https://api.github.com";

/// GitHub provider adapter.
#[derive(Debug, Clone)]
pub struct GitHubProvider {
    http: reqwest::Client,
    api_base: String,
    token: Option<String>,
}

impl GitHubProvider {
    /// Build a provider with an optional personal access token.
    ///
    /// # Errors
    /// When the HTTP client cannot be constructed.
    pub fn new(token: Option<String>) -> Result<Self, ProviderError> {
        Self::with_api_base(token, DEFAULT_API_BASE.to_string())
    }

    /// Build a provider against a custom API base (tests, GHES).
    ///
    /// # Errors
    /// When the HTTP client cannot be constructed.
    pub fn with_api_base(token: Option<String>, api_base: String) -> Result<Self, ProviderError> {
        let http = reqwest::Client::builder()
            .user_agent("gitspin")
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| ProviderError::Request(e.to_string()))?;
        Ok(Self {
            http,
            api_base: api_base.trim_end_matches('/').to_string(),
            token,
        })
    }

    fn get(&self, path: &str, query: &[(&str, &str)]) -> reqwest::RequestBuilder {
        let mut req = self
            .http
            .get(format!("{}{path}", self.api_base))
            .query(query);
        if let Some(token) = &self.token {
            req = req.bearer_auth(token);
        }
        req
    }

    async fn fetch_commits_page(
        &self,
        repo: &RepoRef,
        window: &TimeWindow,
    ) -> Result<Vec<Value>, ProviderError> {
        let path = format!("/repos/{}/commits", repo.full_name());
        let mut query: Vec<(&str, &str)> = vec![("per_page", "100")];
        if let Some(since) = &window.since {
            query.push(("since", since));
        }
        if let Some(until) = &window.until {
            query.push(("until", until));
        }
        let response = self
            .get(&path, &query)
            .send()
            .await
            .map_err(|e| ProviderError::Request(e.to_string()))?;
        if !response.status().is_success() {
            return Err(ProviderError::Request(format!(
                "GET {path} -> {}",
                response.status()
            )));
        }
        response
            .json::<Vec<Value>>()
            .await
            .map_err(|e| ProviderError::Response(e.to_string()))
    }

    async fn fetch_reviews_page(&self, repo: &RepoRef) -> Result<Vec<Value>, ProviderError> {
        let path = format!("/repos/{}/pulls/reviews", repo.full_name());
        let response = self
            .get(&path, &[("per_page", "100")])
            .send()
            .await
            .map_err(|e| ProviderError::Request(e.to_string()))?;
        if !response.status().is_success() {
            return Err(ProviderError::Request(format!(
                "GET {path} -> {}",
                response.status()
            )));
        }
        response
            .json::<Vec<Value>>()
            .await
            .map_err(|e| ProviderError::Response(e.to_string()))
    }
}

/// Map GitHub commit JSON to the neutral [`CommitSummary`].
#[must_use]
pub fn parse_commit(value: &Value) -> Option<CommitSummary> {
    let id = value.get("sha")?.as_str()?.to_string();
    let commit = value.get("commit")?;
    let message = commit
        .get("message")?
        .as_str()?
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    let author = commit
        .get("author")
        .and_then(|a| a.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string);
    Some(CommitSummary {
        id,
        author,
        message,
        // GitHub's list-commits response does not carry stats; counts stay 0
        // until the dedicated stats fetch lands.
        additions: 0,
        deletions: 0,
    })
}

/// Map a GitHub PR review JSON object to the neutral [`ReviewSummary`].
#[must_use]
pub fn parse_review(value: &Value) -> Option<ReviewSummary> {
    let pr_number = value
        .pointer("/pull_request/number")
        .and_then(Value::as_u64)?;
    let reviewer = value
        .pointer("/user/login")
        .and_then(Value::as_str)?
        .to_string();
    Some(ReviewSummary {
        pr_number,
        reviewer,
        state: normalize_review_state(value.get("state")?.as_str()?),
        submitted_at: value
            .get("submitted_at")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

/// Normalize GitHub review state to the forge-neutral vocabulary.
#[must_use]
pub fn normalize_review_state(state: &str) -> String {
    match state {
        "APPROVED" => "approved".into(),
        "CHANGES_REQUESTED" => "changes_requested".into(),
        "COMMENTED" => "commented".into(),
        other => other.to_ascii_lowercase(),
    }
}

#[async_trait]
impl ContributionProvider for GitHubProvider {
    fn name(&self) -> &'static str {
        "github"
    }

    async fn contributors(
        &self,
        repos: &[RepoRef],
        window: &TimeWindow,
    ) -> Result<Vec<ContributorSummary>, ProviderError> {
        let commits = self.commits(repos, window).await?;
        let mut counts: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
        for commit in commits {
            if let Some(author) = commit.author {
                *counts.entry(author).or_default() += 1;
            }
        }
        Ok(counts
            .into_iter()
            .map(|(login, commit_count)| ContributorSummary {
                login,
                commit_count,
            })
            .collect())
    }

    async fn commits(
        &self,
        repos: &[RepoRef],
        window: &TimeWindow,
    ) -> Result<Vec<CommitSummary>, ProviderError> {
        let mut out = Vec::new();
        for repo in repos {
            for value in self.fetch_commits_page(repo, window).await? {
                if let Some(commit) = parse_commit(&value) {
                    out.push(commit);
                }
            }
        }
        Ok(out)
    }

    async fn lines_changed(
        &self,
        _repos: &[RepoRef],
        _window: &TimeWindow,
    ) -> Result<LinesChanged, ProviderError> {
        // Deferred: per-commit stats require an extra fetch per commit.
        Err(ProviderError::NotImplemented("github lines_changed".into()))
    }

    async fn reviews(
        &self,
        repos: &[RepoRef],
        _window: &TimeWindow,
    ) -> Result<Vec<ReviewSummary>, ProviderError> {
        let mut out = Vec::new();
        for repo in repos {
            for value in self.fetch_reviews_page(repo).await? {
                if let Some(review) = parse_review(&value) {
                    out.push(review);
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_commit_extracts_fields() {
        let raw = json!({
            "sha": "abc123",
            "commit": {
                "message": "fix: parser\n\nbody",
                "author": { "name": "shiv" }
            }
        });
        let commit = parse_commit(&raw).expect("commit");
        assert_eq!(commit.id, "abc123");
        assert_eq!(commit.author.as_deref(), Some("shiv"));
        assert_eq!(commit.message, "fix: parser");
    }

    #[test]
    fn parse_commit_rejects_malformed() {
        assert!(parse_commit(&json!({ "nope": 1 })).is_none());
    }

    #[test]
    fn parse_review_extracts_fields() {
        let raw = json!({
            "state": "APPROVED",
            "user": { "login": "alice" },
            "submitted_at": "2026-11-01T00:00:00Z",
            "pull_request": { "number": 7 }
        });
        let review = parse_review(&raw).expect("review");
        assert_eq!(review.pr_number, 7);
        assert_eq!(review.reviewer, "alice");
        assert_eq!(review.state, "approved");
    }

    #[test]
    fn review_state_normalization() {
        assert_eq!(
            normalize_review_state("CHANGES_REQUESTED"),
            "changes_requested"
        );
        assert_eq!(normalize_review_state("UNKNOWN"), "unknown");
    }

    #[test]
    fn provider_name_is_stable() {
        let provider = GitHubProvider::new(None).expect("provider");
        assert_eq!(provider.name(), "github");
    }
}

//! GitHub adapter: read-only REST implementation of `ContributionProvider`.
//!
//! Minimal by design — three read surfaces (commits, contributors, PR
//! reviews) plus pure response parsers that are unit-tested without network.
//! PR reviews are assembled by enumerating the repository's pull requests and
//! reading each numbered `/pulls/{n}/reviews` collection; GitHub has no
//! repository-wide reviews route.

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

    /// Enumerate the repository's pull requests (all states, first page).
    async fn fetch_pull_numbers(&self, repo: &RepoRef) -> Result<Vec<u64>, ProviderError> {
        let path = format!("/repos/{}/pulls", repo.full_name());
        let response = self
            .get(&path, &[("state", "all"), ("per_page", "100")])
            .send()
            .await
            .map_err(|e| ProviderError::Request(e.to_string()))?;
        if !response.status().is_success() {
            return Err(ProviderError::Request(format!(
                "GET {path} -> {}",
                response.status()
            )));
        }
        let values = response
            .json::<Vec<Value>>()
            .await
            .map_err(|e| ProviderError::Response(e.to_string()))?;
        Ok(values
            .iter()
            .filter_map(|v| v.get("number").and_then(Value::as_u64))
            .collect())
    }

    /// Fetch the review collection of one numbered pull request.
    async fn fetch_pr_reviews(
        &self,
        repo: &RepoRef,
        pull_number: u64,
    ) -> Result<Vec<Value>, ProviderError> {
        let path = format!("/repos/{}/pulls/{pull_number}/reviews", repo.full_name());
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
///
/// The author is the forge login from the top-level `author` GitHub user;
/// `commit.author.name` is a free-form git display name and never a login,
/// so commits whose top-level author is null stay unattributed.
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
    let author = value
        .get("author")
        .and_then(|a| a.get("login"))
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
///
/// `pr_number` is carried from retrieval: review rows identify their pull
/// request only by URL, so the numbered collection they were fetched from
/// supplies the number.
#[must_use]
pub fn parse_review(value: &Value, pr_number: u64) -> Option<ReviewSummary> {
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
            if let Some(login) = commit.author {
                *counts.entry(login).or_default() += 1;
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
            for pull_number in self.fetch_pull_numbers(repo).await? {
                for value in self.fetch_pr_reviews(repo, pull_number).await? {
                    if let Some(review) = parse_review(&value, pull_number) {
                        out.push(review);
                    }
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
            "author": { "login": "shiv" },
            "commit": {
                "message": "fix: parser\n\nbody",
                "author": { "name": "Shiv Rossi" }
            }
        });
        let commit = parse_commit(&raw).expect("commit");
        assert_eq!(commit.id, "abc123");
        // Author is the forge login, not the git display name.
        assert_eq!(commit.author.as_deref(), Some("shiv"));
        assert_eq!(commit.message, "fix: parser");
    }

    #[test]
    fn parse_commit_unattributed_author_stays_none() {
        // Null top-level GitHub author: the free-form git name is NOT a login
        // and must not be attributed.
        let raw = json!({
            "sha": "def456",
            "author": serde_json::Value::Null,
            "commit": {
                "message": "docs: tweak",
                "author": { "name": "Ghost Writer" }
            }
        });
        let commit = parse_commit(&raw).expect("commit");
        assert_eq!(commit.author, None);
    }

    #[test]
    fn parse_commit_rejects_malformed() {
        assert!(parse_commit(&json!({ "nope": 1 })).is_none());
    }

    #[test]
    fn parse_review_extracts_native_shape() {
        // Native /pulls/{n}/reviews row: the pull request appears only as a
        // URL — there is no nested `pull_request.number` object.
        let raw = json!({
            "state": "APPROVED",
            "user": { "login": "alice" },
            "submitted_at": "2026-11-01T00:00:00Z",
            "pull_request_url": "https://api.github.com/repos/acme/widget/pulls/7"
        });
        let review = parse_review(&raw, 7).expect("review");
        assert_eq!(review.pr_number, 7);
        assert_eq!(review.reviewer, "alice");
        assert_eq!(review.state, "approved");
    }

    #[test]
    fn parse_review_needs_no_invented_field() {
        // A native row with no pull-request reference at all still parses
        // when the retrieval context carries the number.
        let raw = json!({
            "state": "CHANGES_REQUESTED",
            "user": { "login": "bob" },
            "submitted_at": "2026-11-02T00:00:00Z"
        });
        let review = parse_review(&raw, 12).expect("review");
        assert_eq!(review.pr_number, 12);
        assert_eq!(review.state, "changes_requested");
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

    /// Minimal loopback HTTP server serving canned JSON by path (query
    /// stripped). Lives until process exit; fine for a test binary.
    fn spawn_loopback(routes: Vec<(String, String)>) -> String {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback");
        let addr = listener.local_addr().expect("loopback addr");
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let Ok(reader) = stream.try_clone() else {
                    continue;
                };
                let mut reader = BufReader::new(reader);
                let mut request_line = String::new();
                if reader.read_line(&mut request_line).is_err() {
                    continue;
                }
                let path = request_line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .split('?')
                    .next()
                    .unwrap_or_default()
                    .to_string();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).is_err() || line == "\r\n" || line == "\n" {
                        break;
                    }
                }
                let (status, reason, body) = routes
                    .iter()
                    .find(|(route, _)| route == &path)
                    .map(|(_, body)| (200u16, "OK", body.clone()))
                    .unwrap_or((404, "Not Found", "{\"message\":\"Not Found\"}".to_string()));
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn reviews_read_numbered_pull_request_routes() {
        let routes = vec![
            (
                "/repos/acme/widget/pulls".to_string(),
                r#"[{"number": 1}, {"number": 7}]"#.to_string(),
            ),
            (
                "/repos/acme/widget/pulls/1/reviews".to_string(),
                r#"[{"state": "APPROVED", "user": {"login": "alice"}, "submitted_at": "2026-11-01T00:00:00Z", "pull_request_url": "https://api.github.com/repos/acme/widget/pulls/1"}]"#.to_string(),
            ),
            (
                "/repos/acme/widget/pulls/7/reviews".to_string(),
                r#"[{"state": "CHANGES_REQUESTED", "user": {"login": "bob"}}]"#.to_string(),
            ),
        ];
        let base = spawn_loopback(routes);
        let provider = GitHubProvider::with_api_base(None, base).expect("provider");
        let reviews = provider
            .reviews(&[RepoRef::new("acme", "widget")], &TimeWindow::unbounded())
            .await
            .expect("reviews");
        assert_eq!(reviews.len(), 2);
        assert_eq!(reviews[0].pr_number, 1);
        assert_eq!(reviews[0].reviewer, "alice");
        assert_eq!(reviews[0].state, "approved");
        assert_eq!(reviews[1].pr_number, 7);
        assert_eq!(reviews[1].reviewer, "bob");
        assert_eq!(reviews[1].state, "changes_requested");
    }

    #[tokio::test]
    async fn contributors_key_by_login_not_display_name() {
        let commit = |sha: &str, login: serde_json::Value, git_name: &str| {
            json!({
                "sha": sha,
                "author": login,
                "commit": { "message": "feat: work", "author": { "name": git_name } }
            })
            .to_string()
        };
        let routes = vec![(
            "/repos/acme/widget/commits".to_string(),
            format!(
                "[{}, {}, {}, {}]",
                // Rename under one login: must stay one contributor.
                commit("a", json!({ "login": "shiv" }), "Shiv Rossi"),
                commit("b", json!({ "login": "shiv" }), "S. Rossi"),
                // Display-name collision across logins: must stay distinct.
                commit("c", json!({ "login": "alice" }), "Shiv Rossi"),
                // Unattributed: counts toward no login.
                commit("d", serde_json::Value::Null, "Ghost Writer")
            ),
        )];
        let base = spawn_loopback(routes);
        let provider = GitHubProvider::with_api_base(None, base).expect("provider");
        let contributors = provider
            .contributors(&[RepoRef::new("acme", "widget")], &TimeWindow::unbounded())
            .await
            .expect("contributors");
        assert_eq!(
            contributors,
            vec![
                ContributorSummary {
                    login: "alice".into(),
                    commit_count: 1,
                },
                ContributorSummary {
                    login: "shiv".into(),
                    commit_count: 2,
                },
            ]
        );
        let commits = provider
            .commits(&[RepoRef::new("acme", "widget")], &TimeWindow::unbounded())
            .await
            .expect("commits");
        assert_eq!(commits.len(), 4);
        assert_eq!(
            commits[3].author, None,
            "unattributed commit stays unattributed"
        );
    }
}

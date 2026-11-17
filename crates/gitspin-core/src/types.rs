//! Core contribution types, forge-neutral by construction.

use serde::{Deserialize, Serialize};

/// A repository reference on any provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoRef {
    /// Owner or namespace (user/org).
    pub owner: String,
    /// Repository name.
    pub name: String,
}

impl RepoRef {
    #[must_use]
    pub fn new(owner: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            owner: owner.into(),
            name: name.into(),
        }
    }

    /// `owner/name` display form.
    #[must_use]
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

/// Inclusive/exclusive time bounds as RFC 3339 timestamps. `None` = unbounded.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeWindow {
    /// Lower bound (RFC 3339), inclusive, or unbounded.
    #[serde(default)]
    pub since: Option<String>,
    /// Upper bound (RFC 3339), exclusive, or unbounded.
    #[serde(default)]
    pub until: Option<String>,
}

impl TimeWindow {
    #[must_use]
    pub fn unbounded() -> Self {
        Self::default()
    }
}

/// Per-contributor commit count over the queried window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContributorSummary {
    /// Forge login.
    pub login: String,
    /// Number of commits attributed to the contributor.
    pub commit_count: u64,
}

/// A single commit, forge-neutral.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitSummary {
    /// Provider-specific commit id (SHA on GitHub).
    pub id: String,
    /// Author login, when attributable.
    pub author: Option<String>,
    /// First line of the commit message.
    pub message: String,
    /// Lines added.
    pub additions: u64,
    /// Lines removed.
    pub deletions: u64,
}

/// Aggregate lines added/removed across the queried repos and window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinesChanged {
    pub added: u64,
    pub removed: u64,
}

/// A PR review event, forge-neutral.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewSummary {
    /// Pull/merge request number on the provider.
    pub pr_number: u64,
    /// Reviewer login.
    pub reviewer: String,
    /// Forge-neutral state string (approved, changes_requested, commented).
    pub state: String,
    /// RFC 3339 timestamp, when available.
    pub submitted_at: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_ref_full_name() {
        assert_eq!(
            RepoRef::new("TechGodHQ", "gitspin").full_name(),
            "TechGodHQ/gitspin"
        );
    }

    #[test]
    fn time_window_default_is_unbounded() {
        let w = TimeWindow::unbounded();
        assert!(w.since.is_none() && w.until.is_none());
    }

    #[test]
    fn time_window_serde_roundtrip() {
        let w = TimeWindow {
            since: Some("2026-01-01T00:00:00Z".into()),
            until: None,
        };
        let json = serde_json::to_string(&w).expect("serialize");
        let back: TimeWindow = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, w);
    }

    #[test]
    fn lines_changed_default_zero() {
        assert_eq!(
            LinesChanged::default(),
            LinesChanged {
                added: 0,
                removed: 0
            }
        );
    }
}

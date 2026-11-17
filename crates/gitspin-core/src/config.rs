//! Configuration model: which provider, which repos.

use crate::types::RepoRef;
use serde::{Deserialize, Serialize};

/// Provider selection. GitLab and Forgejo are declared but deferred.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ProviderConfig {
    /// GitHub adapter (implemented, read-only REST).
    GitHub {
        /// Personal access token; when absent, unauthenticated requests only.
        #[serde(default)]
        token: Option<String>,
    },
    /// GitLab adapter — deferred.
    GitLab,
    /// Forgejo adapter — deferred.
    Forgejo,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self::GitHub { token: None }
    }
}

impl ProviderConfig {
    /// Stable identifier matching [`crate::provider::ContributionProvider::name`].
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::GitHub { .. } => "github",
            Self::GitLab => "gitlab",
            Self::Forgejo => "forgejo",
        }
    }
}

/// Top-level gitspin configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitspinConfig {
    /// Which forge to query.
    #[serde(default)]
    pub provider: ProviderConfig,
    /// Repositories to aggregate over.
    #[serde(default)]
    pub repos: Vec<RepoRef>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_github_without_token() {
        let cfg = GitspinConfig::default();
        assert_eq!(cfg.provider.name(), "github");
        assert!(cfg.repos.is_empty());
    }

    #[test]
    fn config_yaml_roundtrip() {
        let cfg = GitspinConfig {
            provider: ProviderConfig::GitHub { token: None },
            repos: vec![RepoRef::new("TechGodHQ", "gitspin")],
        };
        let yaml = serde_yaml::to_string(&cfg).expect("serialize");
        let back: GitspinConfig = serde_yaml::from_str(&yaml).expect("deserialize");
        assert_eq!(back, cfg);
    }

    #[test]
    fn deferred_providers_are_named() {
        assert_eq!(ProviderConfig::GitLab.name(), "gitlab");
        assert_eq!(ProviderConfig::Forgejo.name(), "forgejo");
    }
}

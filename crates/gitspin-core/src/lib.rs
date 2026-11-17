//! gitspin-core: the provider-agnostic domain model.
//!
//! Nothing here knows about a specific git forge. Concrete adapters
//! (GitHub today; GitLab, Forgejo deferred) implement [`provider::ContributionProvider`].

pub mod config;
pub mod provider;
pub mod types;

pub use config::{GitspinConfig, ProviderConfig};
pub use provider::{ContributionProvider, ProviderError};
pub use types::{
    CommitSummary, ContributorSummary, LinesChanged, RepoRef, ReviewSummary, TimeWindow,
};

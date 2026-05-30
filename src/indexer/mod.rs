pub mod extract;
pub mod git;
pub mod progress;
pub mod target;
pub mod service;
pub mod watcher;
pub mod scanner;

pub use git::{build_source_urn, resolve_git_state, GitState};
pub use progress::{IndexProgress, TargetProgress};
pub use target::{IngestionTarget, TargetType};
pub use service::{IndexService, IngestionEvent};
pub use watcher::IndexWatcher;
pub use scanner::{scan_target, is_likely_binary};

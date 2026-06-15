pub mod extract;
pub mod git;
pub mod progress;
pub mod scanner;
pub mod service;
pub mod target;
pub mod watcher;

pub use git::{GitState, resolve_git_state};
pub use progress::{IndexProgress, TargetProgress};
pub use scanner::{is_likely_binary, scan_target};
pub use service::{IndexService, IngestionEvent};
pub use target::{IngestionTarget, TargetType};
pub use watcher::IndexWatcher;

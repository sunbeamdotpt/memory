use serde::{Deserialize, Serialize};

/// A filesystem path or directory that the indexer should watch and ingest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IngestionTarget {
    pub id: String,
    pub path: String,
    pub target_type: TargetType,
    pub namespace: String,
    pub enabled: bool,
    pub created_at: i64,
    pub last_scan_at: Option<i64>,
    pub last_scan_git_branch: Option<String>,
    pub last_scan_git_commit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TargetType {
    File,
    Directory,
    GitRepo,
}

impl TargetType {
    pub fn as_str(&self) -> &'static str {
        match self {
            TargetType::File => "file",
            TargetType::Directory => "directory",
            TargetType::GitRepo => "git_repo",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "file" => Some(TargetType::File),
            "directory" => Some(TargetType::Directory),
            "git_repo" => Some(TargetType::GitRepo),
            _ => None,
        }
    }
}

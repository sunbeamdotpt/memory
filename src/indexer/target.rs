use serde::{Deserialize, Serialize};
use std::str::FromStr;

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
}

impl FromStr for TargetType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "file" => Ok(TargetType::File),
            "directory" => Ok(TargetType::Directory),
            "git_repo" => Ok(TargetType::GitRepo),
            _ => Err(format!("unknown target type: {s}")),
        }
    }
}

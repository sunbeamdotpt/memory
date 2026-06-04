use crate::error::{Result, ServerError};
use std::path::{Path, PathBuf};

/// Resolved git state for a file inside a repository.
#[derive(Debug, Clone)]
pub struct GitState {
    pub host: String,
    pub org: Option<String>,
    pub repo: String,
    pub branch: String,
    pub commit: String,
    pub repo_root: PathBuf,
}

/// Attempt to discover a git repository containing `path` and resolve
/// remote, branch, and HEAD commit information.
pub fn resolve_git_state(path: &Path) -> Result<Option<GitState>> {
    let repo = match gix::discover(path) {
        Ok(r) => r,
        Err(_) => return Ok(None),
    };

    let repo_root = repo.work_dir().map(Path::to_path_buf).unwrap_or_default();

    // Resolve branch name
    let branch = match repo.head_ref().map_err(|e| ServerError::DatabaseError(e.to_string()))? {
        Some(r) => r.name().shorten().to_string(),
        None => "HEAD".to_string(),
    };

    // Resolve HEAD commit
    let commit = repo
        .head_commit().map_err(|e| ServerError::DatabaseError(e.to_string()))?
        .id
        .to_hex()
        .to_string();

    // Resolve remote URL
    let (host, org, repo_name) = match repo.find_remote("origin") {
        Ok(remote) => {
            let url = remote
                .url(gix::remote::Direction::Fetch)
                .map(|u| u.to_bstring().to_string());
            match url {
                Some(url_str) => parse_git_url(&url_str),
                None => ("local".to_string(), None, "repo".to_string()),
            }
        }
        Err(_) => ("local".to_string(), None, "repo".to_string()),
    };

    Ok(Some(GitState {
        host,
        org,
        repo: repo_name,
        branch,
        commit,
        repo_root,
    }))
}

/// Parse a git remote URL into (host, org, repo).
///
/// Supports:
/// - `https://github.com/acme/repo.git`
/// - `https://github.com/acme/repo`
/// - `git@github.com:acme/repo.git`
/// - `git@github.com:acme/repo`
/// - `ssh://git@github.com/acme/repo.git`
fn parse_git_url(url: &str) -> (String, Option<String>, String) {
    // HTTPS: https://github.com/acme/repo.git
    if let Some(rest) = url.strip_prefix("https://") {
        return parse_path_style(rest);
    }
    if let Some(rest) = url.strip_prefix("http://") {
        return parse_path_style(rest);
    }

    // SSH: git@github.com:acme/repo.git
    if let Some(at_idx) = url.find('@') {
        let after_at = &url[at_idx + 1..];
        if let Some(colon_idx) = after_at.find(':') {
            let host = &after_at[..colon_idx];
            let path = &after_at[colon_idx + 1..];
            let (org, repo) = parse_org_repo(path);
            return (host.to_string(), org, repo);
        }
    }

    // ssh://git@github.com/acme/repo.git
    if let Some(rest) = url.strip_prefix("ssh://") {
        return parse_path_style(rest);
    }

    // Fallback: try path-style
    parse_path_style(url)
}

fn parse_path_style(input: &str) -> (String, Option<String>, String) {
    // input: github.com/acme/repo.git
    let without_git = input.strip_suffix(".git").unwrap_or(input);
    let parts: Vec<&str> = without_git.split('/').collect();
    match parts.len() {
        0 | 1 => (input.to_string(), None, "repo".to_string()),
        2 => (parts[0].to_string(), None, parts[1].to_string()),
        _ => {
            let host = parts[0].to_string();
            let org = Some(parts[1].to_string());
            let repo = parts[2].to_string();
            (host, org, repo)
        }
    }
}

fn parse_org_repo(path: &str) -> (Option<String>, String) {
    let without_git = path.strip_suffix(".git").unwrap_or(path);
    let parts: Vec<&str> = without_git.split('/').collect();
    match parts.len() {
        0 => (None, "repo".to_string()),
        1 => (None, parts[0].to_string()),
        _ => (Some(parts[0].to_string()), parts[1].to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_git_url_https() {
        let (host, org, repo) = parse_git_url("https://github.com/acme/repo.git");
        assert_eq!(host, "github.com");
        assert_eq!(org, Some("acme".to_string()));
        assert_eq!(repo, "repo");
    }

    #[test]
    fn test_parse_git_url_ssh() {
        let (host, org, repo) = parse_git_url("git@github.com:acme/repo.git");
        assert_eq!(host, "github.com");
        assert_eq!(org, Some("acme".to_string()));
        assert_eq!(repo, "repo");
    }

    #[test]
    fn test_parse_git_url_no_org() {
        let (host, org, repo) = parse_git_url("https://git.example.com/repo");
        assert_eq!(host, "git.example.com");
        assert_eq!(org, None);
        assert_eq!(repo, "repo");
    }
}

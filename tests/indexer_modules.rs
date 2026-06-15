use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use crossbeam_channel::bounded;
use sunbeam_memory::config::MemoryConfig;
use sunbeam_memory::indexer::{
    IndexProgress, IndexService, IndexWatcher, TargetProgress, TargetType, extract::extract_text,
    is_likely_binary, resolve_git_state, scan_target,
};
use sunbeam_memory::memory::service::MemoryService;
use sunbeam_memory::urn::{ContentType, Origin, SourceUrn, invalid_urn_response, schema_json};

// ── helpers ───────────────────────────────────────────────────────────────────

fn init_git_repo(
    root: &Path,
    remote_url: Option<&str>,
    initial_branch: &str,
) -> std::path::PathBuf {
    let repo_dir = root.join("repo");
    fs::create_dir_all(&repo_dir).unwrap();

    let output = Command::new("git")
        .arg("init")
        .arg("--initial-branch")
        .arg(initial_branch)
        .arg(&repo_dir)
        .output()
        .expect("git init failed");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    if let Some(url) = remote_url {
        let output = Command::new("git")
            .arg("-C")
            .arg(&repo_dir)
            .arg("remote")
            .arg("add")
            .arg("origin")
            .arg(url)
            .output()
            .expect("git remote add failed");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Configure committer so we can make a commit.
    for (key, val) in [("user.name", "Test"), ("user.email", "test@example.com")] {
        let output = Command::new("git")
            .arg("-C")
            .arg(&repo_dir)
            .arg("config")
            .arg(key)
            .arg(val)
            .output()
            .expect("git config failed");
        assert!(output.status.success());
    }

    let file = repo_dir.join("README.md");
    fs::write(&file, "# test repo\n").unwrap();

    let output = Command::new("git")
        .arg("-C")
        .arg(&repo_dir)
        .arg("add")
        .arg(".")
        .output()
        .expect("git add failed");
    assert!(output.status.success());

    let output = Command::new("git")
        .arg("-C")
        .arg(&repo_dir)
        .arg("commit")
        .arg("-m")
        .arg("initial")
        .output()
        .expect("git commit failed");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    repo_dir
}

async fn setup() -> (MemoryService, IndexService, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let config = MemoryConfig {
        base_dir: root.to_str().unwrap().to_string(),
        ..Default::default()
    };
    let memory = MemoryService::new(&config).await.unwrap();
    let (dummy_tx, dummy_rx) = bounded(1);
    let watcher = IndexWatcher::new(dummy_tx).unwrap();
    let indexer = IndexService::new(memory.clone(), dummy_rx, watcher);
    (memory, indexer, dir)
}

// ── git.rs ────────────────────────────────────────────────────────────────────

#[test]
fn test_resolve_git_state_https_remote() {
    let dir = tempfile::tempdir().unwrap();
    let repo = init_git_repo(dir.path(), Some("https://github.com/acme/repo.git"), "main");
    let state = resolve_git_state(&repo).unwrap().expect("should resolve");
    assert_eq!(state.host, "github.com");
    assert_eq!(state.org, Some("acme".to_string()));
    assert_eq!(state.repo, "repo");
    assert_eq!(state.branch, "main");
    assert_eq!(state.commit.len(), 40);
    assert_eq!(state.repo_root, repo);
}

#[test]
fn test_resolve_git_state_ssh_remote() {
    let dir = tempfile::tempdir().unwrap();
    let repo = init_git_repo(dir.path(), Some("git@github.com:acme/repo.git"), "main");
    let state = resolve_git_state(&repo).unwrap().expect("should resolve");
    assert_eq!(state.host, "github.com");
    assert_eq!(state.org, Some("acme".to_string()));
    assert_eq!(state.repo, "repo");
}

#[test]
fn test_resolve_git_state_ssh_url_remote() {
    let dir = tempfile::tempdir().unwrap();
    let repo = init_git_repo(
        dir.path(),
        Some("ssh://github.com/acme/repo.git"),
        "develop",
    );
    let state = resolve_git_state(&repo).unwrap().expect("should resolve");
    assert_eq!(state.host, "github.com");
    assert_eq!(state.org, Some("acme".to_string()));
    assert_eq!(state.repo, "repo");
    assert_eq!(state.branch, "develop");
}

#[test]
fn test_resolve_git_state_http_remote_no_org() {
    let dir = tempfile::tempdir().unwrap();
    let repo = init_git_repo(dir.path(), Some("http://git.example.com/repo.git"), "main");
    let state = resolve_git_state(&repo).unwrap().expect("should resolve");
    assert_eq!(state.host, "git.example.com");
    assert_eq!(state.org, None);
    assert_eq!(state.repo, "repo");
}

#[test]
fn test_resolve_git_state_no_remote_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let repo = init_git_repo(dir.path(), None, "main");
    let state = resolve_git_state(&repo).unwrap().expect("should resolve");
    assert_eq!(state.host, "local");
    assert_eq!(state.org, None);
    assert_eq!(state.repo, "repo");
}

#[test]
fn test_resolve_git_state_outside_repo() {
    let dir = tempfile::tempdir().unwrap();
    let nongit = dir.path().join("not-a-repo");
    fs::create_dir_all(&nongit).unwrap();
    assert!(resolve_git_state(&nongit).unwrap().is_none());
}

// ── scanner.rs ────────────────────────────────────────────────────────────────

#[test]
fn test_is_likely_binary_by_extension() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("app.exe");
    fs::write(&bin, "this is plain text").unwrap();
    assert!(is_likely_binary(&bin));
}

#[test]
fn test_is_likely_binary_magic_bytes() {
    let dir = tempfile::tempdir().unwrap();

    let cases: Vec<(&str, &[u8])> = vec![
        ("img.png", b"\x89PNG\r\n\x1a\n"),
        ("img.jpg", &[0xFF, 0xD8, 0xFF, 0xE0]),
        ("img.gif", b"GIF87a"),
        ("file.webp", b"RIFF\x00\x00\x00\x00WEBP"),
        ("file.zip", b"PK\x03\x04"),
        ("file.gz", &[0x1F, 0x8B, 0x08, 0x00]),
        ("file.elf", &[0x7F, b'E', b'L', b'F']),
        ("file.pdf", b"%PDF-1.4"),
        ("file.mp3", b"ID3\x03"),
        ("file.class", &[0xCA, 0xFE, 0xBA, 0xBE]),
    ];

    for (name, header) in cases {
        let path = dir.path().join(name);
        fs::write(&path, header).unwrap();
        assert!(is_likely_binary(&path), "{} should be binary", name);
    }
}

#[test]
fn test_is_likely_binary_null_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("binary.bin");
    fs::write(&path, b"hello\x00world").unwrap();
    assert!(is_likely_binary(&path));
}

#[test]
fn test_is_likely_binary_empty_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.txt");
    fs::write(&path, "").unwrap();
    assert!(!is_likely_binary(&path));
}

#[test]
fn test_is_likely_binary_unreadable_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secret.txt");
    fs::write(&path, "not readable").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();

    assert!(!is_likely_binary(&path));

    // Restore so TempDir can clean up on macOS.
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
}

#[test]
fn test_scan_target_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.txt");
    fs::write(&file, "hi").unwrap();
    let files = scan_target(&file).unwrap();
    assert_eq!(files, vec![file]);
}

#[test]
fn test_scan_target_directory() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("one.txt"), "one").unwrap();
    let sub = dir.path().join("sub");
    fs::create_dir(&sub).unwrap();
    fs::write(sub.join("two.txt"), "two").unwrap();

    let mut files = scan_target(dir.path()).unwrap();
    files.sort();
    assert_eq!(files.len(), 2);
    assert!(
        files
            .iter()
            .all(|p| p.file_name().unwrap() == "one.txt" || p.file_name().unwrap() == "two.txt")
    );
}

#[test]
fn test_scan_target_skips_git_directory() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("file.txt"), "ok").unwrap();
    let git = dir.path().join(".git");
    fs::create_dir(&git).unwrap();
    fs::write(git.join("config"), "secret").unwrap();

    let files = scan_target(dir.path()).unwrap();
    assert_eq!(files.len(), 1);
    assert!(files[0].ends_with("file.txt"));
}

#[test]
fn test_scan_target_nonexistent_returns_empty() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("does-not-exist");
    let files = scan_target(&missing).unwrap();
    assert!(files.is_empty());
}

// ── progress.rs ───────────────────────────────────────────────────────────────

#[test]
fn test_index_progress_set_get_remove_clear_to_vec() {
    let progress = IndexProgress::new();
    assert!(progress.get("t1").is_none());
    assert!(progress.to_vec().is_empty());

    let p = TargetProgress {
        files_total: 10,
        files_pending: 5,
        files_processing: 2,
        files_completed: 3,
        files_failed: 0,
        current_file: Some("foo.rs".to_string()),
        last_error: None,
    };
    progress.set("t1", p.clone());

    let got = progress.get("t1").unwrap();
    assert_eq!(got.files_total, 10);
    assert_eq!(got.files_pending, 5);
    assert_eq!(got.current_file, Some("foo.rs".to_string()));

    progress.update("t1", |tp| tp.files_completed = 4);
    assert_eq!(progress.get("t1").unwrap().files_completed, 4);

    let vec = progress.to_vec();
    assert_eq!(vec.len(), 1);
    assert_eq!(vec[0].0, "t1");

    progress.remove("t1");
    assert!(progress.get("t1").is_none());

    progress.set("t2", TargetProgress::default());
    progress.clear();
    assert!(progress.to_vec().is_empty());
}

// ── target.rs ─────────────────────────────────────────────────────────────────

#[test]
fn test_target_type_as_str_and_from_str() {
    let cases = [
        (TargetType::File, "file"),
        (TargetType::Directory, "directory"),
        (TargetType::GitRepo, "git_repo"),
    ];
    for (variant, s) in cases {
        assert_eq!(variant.as_str(), s);
        assert_eq!(TargetType::from_str(s), Some(variant));
    }
    assert!(TargetType::from_str("unknown").is_none());
}

// ── integration through IndexService ──────────────────────────────────────────

#[tokio::test]
async fn test_add_git_repo_target_and_sync() {
    let (memory, indexer, dir) = setup().await;
    let repo = init_git_repo(dir.path(), Some("https://github.com/acme/repo.git"), "main");

    let source_file = repo.join("src/lib.rs");
    fs::create_dir_all(source_file.parent().unwrap()).unwrap();
    fs::write(&source_file, "git repo indexer content").unwrap();

    let output = Command::new("git")
        .arg("-C")
        .arg(&repo)
        .arg("add")
        .arg(".")
        .output()
        .expect("git add failed");
    assert!(output.status.success());

    let output = Command::new("git")
        .arg("-C")
        .arg(&repo)
        .arg("commit")
        .arg("-m")
        .arg("add source")
        .output()
        .expect("git commit failed");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let ids = indexer
        .add_target(repo.to_str().unwrap(), Some("default"), Some("git_repo"))
        .await
        .unwrap();
    assert_eq!(ids.len(), 1);

    indexer.sync_target(&ids[0]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;

    let results = memory
        .search_facts("git repo indexer content", 5, None)
        .await
        .unwrap();
    assert!(!results.is_empty(), "expected fact from git repo target");

    let targets = indexer.list_targets().await.unwrap();
    let target = targets.into_iter().find(|t| t.id == ids[0]).unwrap();
    assert_eq!(target.target_type, TargetType::GitRepo);
    assert_eq!(target.last_scan_git_branch, Some("main".to_string()));
    assert!(
        target
            .last_scan_git_commit
            .map(|c| c.len() == 40)
            .unwrap_or(false)
    );
}

#[tokio::test]
async fn test_auto_detect_git_repo_target_type() {
    let (_memory, indexer, dir) = setup().await;
    let repo = init_git_repo(dir.path(), None, "main");

    let ids = indexer
        .add_target(repo.to_str().unwrap(), None, None)
        .await
        .unwrap();
    assert_eq!(ids.len(), 1);

    let targets = indexer.list_targets().await.unwrap();
    assert_eq!(targets[0].target_type, TargetType::GitRepo);
}

// ── extract.rs ────────────────────────────────────────────────────────────────

#[test]
fn test_extract_text_plain_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.txt");
    fs::write(&path, "hello world").unwrap();
    let text = extract_text(&path).unwrap();
    assert_eq!(text, "hello world");
}

#[test]
fn test_extract_text_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing.txt");
    let err = extract_text(&path).unwrap_err().to_string();
    assert!(err.contains("failed to read"));
}

#[test]
fn test_extract_text_invalid_pdf() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.pdf");
    fs::write(&path, b"not a pdf").unwrap();
    let err = extract_text(&path).unwrap_err().to_string();
    assert!(err.contains("failed to open PDF"));
}

// ── urn.rs ────────────────────────────────────────────────────────────────────

#[test]
fn test_content_type_and_origin_roundtrip() {
    let content_types = [
        ("code", ContentType::Code),
        ("doc", ContentType::Doc),
        ("web", ContentType::Web),
        ("data", ContentType::Data),
        ("note", ContentType::Note),
        ("conf", ContentType::Conf),
    ];
    let origins = [
        ("git", Origin::Git),
        ("fs", Origin::Fs),
        ("https", Origin::Https),
        ("http", Origin::Http),
        ("db", Origin::Db),
        ("api", Origin::Api),
        ("manual", Origin::Manual),
    ];

    for (type_str, _) in &content_types {
        for (origin_str, _) in &origins {
            let locator = format!("{}/{}", origin_str, type_str);
            let urn = SourceUrn::build(type_str, origin_str, &locator, None).unwrap();
            let parsed = SourceUrn::parse(&urn).unwrap();
            assert_eq!(parsed.content_type.as_str(), *type_str);
            assert_eq!(parsed.origin.as_str(), *origin_str);
        }
    }
}

#[test]
fn test_urn_build_validation_edge_cases() {
    assert!(SourceUrn::build("code", "fs", "", None).is_err());
    assert!(SourceUrn::build("code", "fs", "/foo", Some("")).is_err());
    assert!(SourceUrn::build("bad", "fs", "/foo", None).is_err());
    assert!(SourceUrn::build("code", "bad", "/foo", None).is_err());
}

#[test]
fn test_urn_parse_validation_edge_cases() {
    assert!(SourceUrn::parse("not:a:urn").is_err());
    assert!(SourceUrn::parse("urn:smem::fs:/foo").is_err());
    assert!(SourceUrn::parse("urn:smem:code::/foo").is_err());
    assert!(SourceUrn::parse("urn:smem:code:fs:").is_err());
    assert!(SourceUrn::parse("urn:smem:code:fs:/foo#").is_err());
}

#[test]
fn test_invalid_urn_response_includes_spec() {
    let input = "urn:smem:bad:fs:/foo";
    let err = SourceUrn::parse(input).unwrap_err();
    let resp = invalid_urn_response(input, &err);
    assert_eq!(resp["valid"], false);
    assert_eq!(resp["input"], input);
    assert!(
        resp["error"]
            .as_str()
            .unwrap()
            .contains("unknown content type")
    );
    assert!(resp["spec"].as_str().unwrap().contains("smem URN format"));
}

#[test]
fn test_schema_json_has_expected_structure() {
    let s = schema_json();
    assert_eq!(
        s["format"],
        "urn:smem:<type>:<origin>:<locator>[#<fragment>]"
    );
    assert_eq!(s["content_types"].as_array().unwrap().len(), 6);
    assert_eq!(s["origins"].as_array().unwrap().len(), 7);
    assert!(!s["examples"].as_array().unwrap().is_empty());
}

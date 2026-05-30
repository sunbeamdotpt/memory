use crate::error::{Result, ServerError};
use crate::indexer::{scanner, target::*, IndexProgress as IndexProgressType};
use crate::memory::service::MemoryService;
use crossbeam_channel::Receiver;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex as TokioMutex;
use ulid::Ulid;

/// Events sent from the watcher thread to the tokio-thread IndexService.
#[derive(Debug, Clone)]
pub enum IngestionEvent {
    Create(PathBuf),
    Modify(PathBuf),
    Delete(PathBuf),
    Rename(PathBuf, PathBuf),
}

impl IngestionEvent {
    pub fn path(&self) -> &PathBuf {
        match self {
            IngestionEvent::Create(p) => p,
            IngestionEvent::Modify(p) => p,
            IngestionEvent::Delete(p) => p,
            IngestionEvent::Rename(_, p) => p,
        }
    }
}

/// The indexing service that consumes filesystem events and manages ingestion.
#[derive(Clone)]
pub struct IndexService {
    memory: MemoryService,
    event_rx: Arc<TokioMutex<Option<Receiver<IngestionEvent>>>>,
    progress: IndexProgressType,
    watcher: Arc<TokioMutex<super::watcher::IndexWatcher>>,
}

impl IndexService {
    pub fn new(memory: MemoryService, event_rx: Receiver<IngestionEvent>, watcher: super::watcher::IndexWatcher) -> Self {
        Self {
            memory,
            event_rx: Arc::new(TokioMutex::new(Some(event_rx))),
            progress: IndexProgressType::new(),
            watcher: Arc::new(TokioMutex::new(watcher)),
        }
    }

    pub fn memory(&self) -> &MemoryService {
        &self.memory
    }

    pub fn progress(&self) -> &IndexProgressType {
        &self.progress
    }

    /// Main run loop. Consumes events from the watcher, debounces, and processes batches.
    /// Takes ownership of the internal receiver; clones can no longer call `run()`.
    pub async fn run(self) {
        let event_rx = match self.event_rx.lock().await.take() {
            Some(rx) => rx,
            None => {
                let err_id = Ulid::new().to_string();
                let _ = self.memory.log_error(&err_id, "indexer", "error", "run() called on a clone — only the original IndexService can run the event loop", None);
                return;
            }
        };

        let mut pending: Vec<IngestionEvent> = Vec::new();
        let mut last_event = std::time::Instant::now();
        const DEBOUNCE_MS: u64 = 500;
        const POLL_MS: u64 = 50;

        loop {
            // Non-blocking check for events
            match event_rx.try_recv() {
                Ok(event) => {
                    pending.push(event);
                    last_event = std::time::Instant::now();
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {
                    if !pending.is_empty() && last_event.elapsed().as_millis() >= DEBOUNCE_MS as u128 {
                        let batch = std::mem::take(&mut pending);
                        self.process_batch(batch).await;
                    } else {
                        tokio::time::sleep(Duration::from_millis(POLL_MS)).await;
                    }
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    if !pending.is_empty() {
                        let batch = std::mem::take(&mut pending);
                        self.process_batch(batch).await;
                    }
                    break;
                }
            }
        }
    }

    /// Process a debounced batch of events.
    async fn process_batch(&self, events: Vec<IngestionEvent>) {
        // Load targets from DB to map paths → target_ids
        let targets = match self.list_targets().await {
            Ok(t) => t,
            Err(e) => {
                let err_id = Ulid::new().to_string();
                let _ = self.memory.log_error(&err_id, "indexer", "error", &format!("failed to list targets: {e}"), None);
                return;
            }
        };

        // Group events by target_id
        let mut by_target: std::collections::HashMap<String, Vec<IngestionEvent>> = std::collections::HashMap::new();
        for event in events {
            let path = event.path().clone();
            for target in &targets {
                if !target.enabled {
                    continue;
                }
                let target_path = PathBuf::from(&target.path);
                if path.starts_with(&target_path) {
                    by_target.entry(target.id.clone()).or_default().push(event);
                    break;
                }
            }
        }

        // Process each target's events
        for (target_id, target_events) in by_target {
            if let Err(e) = self.process_target_events(&target_id, target_events).await {
                let err_id = Ulid::new().to_string();
                let _ = self.memory.log_error(&err_id, "indexer", "error", &format!("error processing target {target_id}"), Some(&e.to_string()));
            }
        }
    }

    /// Process events for a single target.
    async fn process_target_events(&self, target_id: &str, events: Vec<IngestionEvent>) -> Result<()> {
        let target = match self.get_target(target_id).await? {
            Some(t) => t,
            None => return Ok(()),
        };

        let target_path = PathBuf::from(&target.path);
        let git_state = crate::indexer::resolve_git_state(&target_path)?;

        for event in events {
            let path = event.path();

            // Skip directories
            if path.is_dir() {
                continue;
            }

            // Skip binary files (simple heuristic)
            if scanner::is_likely_binary(path) {
                continue;
            }

            match event {
                IngestionEvent::Delete(_) => {
                    // Mark fact as stale by looking up its URN
                    let urn = if let Some(ref git) = git_state {
                        let rel = path.strip_prefix(&git.repo_root).unwrap_or(&path);
                        crate::urn::SourceUrn::build_git_urn(
                            &git.host, git.org.as_deref(), &git.repo,
                            &git.branch, &rel.to_string_lossy().replace('\\', "/"),
                            None,
                        ).unwrap_or_else(|_| format!("urn:smem:code:fs:{}", path.display()))
                    } else {
                        format!("urn:smem:code:fs:{}", path.display())
                    };

                    let store = self.memory.get_store().clone();
                    let db = store.db();
                    let db_guard = db.lock().unwrap();
                    if let Some(fact) = db_guard.get_fact_by_source(&urn)? {
                        db_guard.mark_fact_stale(&fact.id)?;
                    }
                }
                _ => {
                    // Create or Modify: read file, hash, ingest
                    if let Err(e) = self.ingest_file(path, &target, git_state.as_ref()).await {
                        let err_id = Ulid::new().to_string();
                        let _ = self.memory.log_error(&err_id, "indexer", "error", &format!("event ingest failed for {}", path.display()), Some(&e.to_string()));
                    }
                }
            }
        }

        Ok(())
    }

    /// Read a file, compute its hash, and add or update the corresponding fact.
    async fn ingest_file(&self, path: &PathBuf, target: &IngestionTarget, git_state: Option<&crate::indexer::GitState>) -> Result<()> {
        let path_for_extract = path.clone();
        let content = match tokio::task::spawn_blocking(move || crate::indexer::extract::extract_text(&path_for_extract)).await {
            Ok(Ok(c)) => c,
            Ok(Err(e)) => return Err(e),
            Err(e) if e.is_cancelled() => return Err(ServerError::DatabaseError("extraction task cancelled".to_string())),
            Err(e) => return Err(ServerError::DatabaseError(format!("extraction task panicked: {e}"))),
        };

        if content.is_empty() {
            return Ok(());
        }

        // Compute content hash
        let _hash = blake3::hash(content.as_bytes()).to_hex().to_string();

        // Build source URN
        let urn = if let Some(git) = git_state {
            let rel = path.strip_prefix(&git.repo_root).unwrap_or(&path);
            crate::urn::SourceUrn::build_git_urn(
                &git.host, git.org.as_deref(), &git.repo,
                &git.branch, &rel.to_string_lossy().replace('\\', "/"),
                None,
            ).unwrap_or_else(|_| format!("urn:smem:code:fs:{}", path.display()))
        } else {
            format!("urn:smem:code:fs:{}", path.display())
        };

        // Check if fact already exists by source URN
        let store = self.memory.get_store().clone();
        let existing_fact = {
            let db = store.db();
            let db_guard = db.lock().unwrap();
            db_guard.get_fact_by_source(&urn)?
        };

        if let Some(fact) = existing_fact {
            // Same branch: update in place (MemoryService handles re-embedding)
            self.memory.update_fact(&fact.id, &content, Some(&urn)).await?;
        } else {
            // New branch or new file: create new fact
            self.memory.add_fact(&target.namespace, &content, Some(&urn)).await?;
        }

        Ok(())
    }

    // ── Target management (called from MCP tools) ─────────────────────────────

    /// Add one or more ingestion targets.
    ///
    /// If `path` contains glob meta-characters (`*`, `?`, `[`), it is expanded
    /// and a target is created for every match.  Otherwise a single target is
    /// created for the literal path.
    pub async fn add_target(&self, path: &str, namespace: Option<&str>, target_type: Option<&str>) -> Result<Vec<String>> {
        // Expand globs if the pattern contains meta-characters
        let paths: Vec<PathBuf> = if is_glob(path) {
            glob::glob(path)
                .map_err(|e| ServerError::InvalidArgument(format!("invalid glob pattern '{}': {}", path, e)))?
                .filter_map(|r| r.ok())
                .filter(|p| p.exists())
                .collect()
        } else {
            vec![PathBuf::from(path)]
        };

        if paths.is_empty() {
            return Err(ServerError::InvalidArgument(format!("glob pattern matched no paths: {}", path)));
        }

        let mut ids = Vec::with_capacity(paths.len());

        for path_buf in paths {
            let path_str = path_buf.to_string_lossy().to_string();
            if !path_buf.exists() {
                continue; // Skip vanished paths
            }

            // Auto-detect target type
            let detected_type = match target_type {
                Some(t) => TargetType::from_str(t).ok_or_else(|| ServerError::InvalidArgument(format!("invalid target_type: {}", t)))?,
                None => {
                    if path_buf.is_file() {
                        TargetType::File
                    } else if path_buf.join(".git").is_dir() {
                        TargetType::GitRepo
                    } else {
                        TargetType::Directory
                    }
                }
            };

            let target = IngestionTarget {
                id: Ulid::new().to_string(),
                path: path_str.clone(),
                target_type: detected_type,
                namespace: namespace.unwrap_or("default").to_string(),
                enabled: true,
                created_at: chrono::Utc::now().timestamp(),
                last_scan_at: None,
                last_scan_git_branch: None,
                last_scan_git_commit: None,
            };

            let id = target.id.clone();
            let store = self.memory.get_store().clone();
            let db = store.db();
            {
                let db_guard = db.lock().unwrap();
                // Check for duplicate path
                if db_guard.get_ingestion_target_by_path(&path_str)?.is_some() {
                    continue; // Skip already-watched paths
                }
                db_guard.add_ingestion_target(&target)?;
            }

            // Start watching the path
            self.watcher.lock().await.add_target(id.clone(), &path_buf)?;
            ids.push(id);
        }

        Ok(ids)
    }

    pub async fn remove_target(&self, target_id: &str) -> Result<bool> {
        let store = self.memory.get_store().clone();
        let db = store.db();
        let db_guard = db.lock().unwrap();
        db_guard.delete_ingestion_target(target_id)
    }

    pub async fn list_targets(&self) -> Result<Vec<IngestionTarget>> {
        let store = self.memory.get_store().clone();
        let db = store.db();
        let db_guard = db.lock().unwrap();
        db_guard.list_ingestion_targets()
    }

    pub async fn get_target(&self, target_id: &str) -> Result<Option<IngestionTarget>> {
        let store = self.memory.get_store().clone();
        let db = store.db();
        let db_guard = db.lock().unwrap();
        db_guard.get_ingestion_target(target_id)
    }

    pub async fn sync_target(&self, target_id: &str) -> Result<()> {
        let target = match self.get_target(target_id).await? {
            Some(t) => t,
            None => return Err(ServerError::NotFound(format!("target not found: {}", target_id))),
        };

        let target_path = PathBuf::from(&target.path);
        let git_state = crate::indexer::resolve_git_state(&target_path)?;

        let files = scanner::scan_target(&target_path)?;

        self.progress.update(target_id, |p| {
            p.files_total = files.len();
            p.files_pending = files.len();
        });

        for (i, file) in files.iter().enumerate() {
            self.progress.update(target_id, |p| {
                p.files_processing = i + 1;
                p.current_file = Some(file.display().to_string());
            });

            if let Err(e) = self.ingest_file(file, &target, git_state.as_ref()).await {
                let err_id = Ulid::new().to_string();
                let _ = self.memory.log_error(&err_id, "indexer", "error", &format!("sync ingest failed for {}", file.display()), Some(&e.to_string()));
                self.progress.update(target_id, |p| {
                    p.files_failed += 1;
                    p.last_error = Some(e.to_string());
                });
            } else {
                self.progress.update(target_id, |p| {
                    p.files_completed += 1;
                });
            }

            self.progress.update(target_id, |p| {
                p.files_pending = p.files_pending.saturating_sub(1);
            });
        }

        self.progress.update(target_id, |p| {
            p.current_file = None;
        });

        // Update scan metadata
        let store = self.memory.get_store().clone();
        let db = store.db();
        let db_guard = db.lock().unwrap();
        db_guard.update_target_scan(
            target_id,
            git_state.as_ref().map(|g| g.branch.as_str()),
            git_state.as_ref().map(|g| g.commit.as_str()),
        )?;

        Ok(())
    }
}

fn is_glob(path: &str) -> bool {
    path.contains('*') || path.contains('?') || path.contains('[')
}

use crate::error::Result;
use crate::indexer::service::IngestionEvent;
use crossbeam_channel::Sender;
use notify::{event::ModifyKind, Event, EventKind, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Platform-agnostic file watcher using `notify` (FSEvents on macOS, inotify on Linux).
pub struct IndexWatcher {
    watcher: notify::RecommendedWatcher,
    targets: Arc<Mutex<HashMap<String, PathBuf>>>,
}

impl IndexWatcher {
    pub fn new(event_tx: Sender<IngestionEvent>) -> Result<Self> {
        let targets = Arc::new(Mutex::new(HashMap::new()));
        let targets_clone = targets.clone();

        let watcher = notify::recommended_watcher(move |res: std::result::Result<Event, notify::Error>| {
            if let Ok(event) = res {
                Self::handle_notify_event(event, &targets_clone, &event_tx);
            }
        })
        .map_err(|e| crate::error::ServerError::DatabaseError(e.to_string()))?;

        Ok(Self { watcher, targets })
    }

    fn handle_notify_event(
        event: Event,
        targets: &Arc<Mutex<HashMap<String, PathBuf>>>,
        tx: &Sender<IngestionEvent>,
    ) {
        let guard = targets.lock().unwrap();

        // Determine event kind
        let kind = match event.kind {
            EventKind::Create(_) => "create",
            EventKind::Modify(ModifyKind::Name(_)) => "rename",
            EventKind::Modify(_) => "modify",
            EventKind::Remove(_) => "delete",
            _ => return, // Ignore other kinds
        };

        // Find matching target(s) for each affected path
        for path in &event.paths {
            for (_target_id, target_path) in guard.iter() {
                if path.starts_with(target_path) {
                    let evt = match kind {
                        "create" => IngestionEvent::Create(path.clone()),
                        "modify" => IngestionEvent::Modify(path.clone()),
                        "delete" => IngestionEvent::Delete(path.clone()),
                        "rename" => {
                            // Rename events have two paths: from and to
                            if event.paths.len() == 2 {
                                let from = event.paths[0].clone();
                                let to = event.paths[1].clone();
                                IngestionEvent::Rename(from, to)
                            } else {
                                // Single-path rename: treat as modify
                                IngestionEvent::Modify(path.clone())
                            }
                        }
                        _ => continue,
                    };
                    let _ = tx.send(evt);
                    break; // Only send to the first matching target
                }
            }
        }
    }

    pub fn add_target(&mut self, target_id: String, path: &Path) -> Result<()> {
        self.targets.lock().unwrap().insert(target_id, path.to_path_buf());
        self.watcher
            .watch(path, RecursiveMode::Recursive)
            .map_err(|e| crate::error::ServerError::DatabaseError(e.to_string()))?;
        Ok(())
    }

    pub fn remove_target(&mut self, target_id: &str) -> Result<()> {
        if let Some(path) = self.targets.lock().unwrap().remove(target_id) {
            let _ = self.watcher.unwatch(&path);
        }
        Ok(())
    }
}

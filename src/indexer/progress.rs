use dashmap::DashMap;
use std::sync::Arc;

/// Per-target ingestion progress, updated concurrently by worker tasks.
#[derive(Debug, Clone, Default)]
pub struct TargetProgress {
    pub files_total: usize,
    pub files_pending: usize,
    pub files_processing: usize,
    pub files_completed: usize,
    pub files_failed: usize,
    pub current_file: Option<String>,
    pub last_error: Option<String>,
}

/// Shared progress store for all targets.
#[derive(Debug, Clone, Default)]
pub struct IndexProgress {
    targets: Arc<DashMap<String, TargetProgress>>,
}

impl IndexProgress {
    pub fn new() -> Self {
        Self {
            targets: Arc::new(DashMap::new()),
        }
    }

    pub fn get(&self, target_id: &str) -> Option<TargetProgress> {
        self.targets.get(target_id).map(|e| e.clone())
    }

    pub fn set(&self, target_id: impl Into<String>, progress: TargetProgress) {
        self.targets.insert(target_id.into(), progress);
    }

    pub fn update<F>(&self, target_id: &str, f: F)
    where
        F: FnOnce(&mut TargetProgress),
    {
        let mut entry = self.targets.entry(target_id.to_string()).or_default();
        f(&mut entry);
    }

    pub fn remove(&self, target_id: &str) {
        self.targets.remove(target_id);
    }

    pub fn clear(&self) {
        self.targets.clear();
    }

    pub fn to_vec(&self) -> Vec<(String, TargetProgress)> {
        self.targets
            .iter()
            .map(|e| (e.key().clone(), e.value().clone()))
            .collect()
    }
}

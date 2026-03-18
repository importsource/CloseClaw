use std::path::PathBuf;

use closeclaw_core::schedule::ScheduleInfo;
use tracing::info;

/// File-based persistence for dynamic schedules.
pub struct ScheduleStore {
    path: PathBuf,
}

impl ScheduleStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Load persisted dynamic schedules. Returns an empty vec if the file
    /// doesn't exist or can't be parsed.
    pub fn load(&self) -> Vec<ScheduleInfo> {
        let content = match std::fs::read_to_string(&self.path) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        serde_json::from_str(&content).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Failed to parse schedules.json, starting fresh");
            Vec::new()
        })
    }

    /// Persist dynamic schedules to disk via atomic write-to-temp-then-rename.
    pub fn save(&self, schedules: &[ScheduleInfo]) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let json = serde_json::to_string_pretty(schedules)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, &json)?;
        std::fs::rename(&tmp, &self.path)?;

        info!(count = schedules.len(), "Persisted dynamic schedules");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_info(id: &str) -> ScheduleInfo {
        ScheduleInfo {
            id: id.to_string(),
            cron: "0 * * * * * *".to_string(),
            agent_id: "default".to_string(),
            message: "test".to_string(),
            source: "dynamic".to_string(),
            notify_peer_id: None,
        }
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScheduleStore::new(dir.path().join("schedules.json"));

        let entries = vec![make_info("a"), make_info("b")];
        store.save(&entries).unwrap();

        let loaded = store.load();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, "a");
        assert_eq!(loaded[1].id, "b");
    }

    #[test]
    fn test_load_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScheduleStore::new(dir.path().join("nope.json"));
        let loaded = store.load();
        assert!(loaded.is_empty());
    }

    #[test]
    fn test_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScheduleStore::new(dir.path().join("schedules.json"));

        store.save(&[make_info("first")]).unwrap();
        store.save(&[make_info("second"), make_info("third")]).unwrap();

        let loaded = store.load();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, "second");
    }

    #[test]
    fn test_load_corrupted_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("schedules.json");
        fs::write(&path, "not json at all").unwrap();

        let store = ScheduleStore::new(path);
        let loaded = store.load();
        assert!(loaded.is_empty());
    }
}

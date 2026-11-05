//! Best-effort cleanup bookkeeping for plugin-owned storage.
//!
//! Rust Core recorded failed cleanups as one `app.storage.cleanup_failed`
//! diagnostics event per batch. The plugin keeps the same fixed-size summary
//! and reports it to the plugin's unified log through
//! [`host::diagnostics`](super::diagnostics): once per batch, carrying only the
//! stage category and the number of failed removals.

/// Fixed-size cleanup summary: batch callers emit once rather than once per file.
#[derive(Debug, Default)]
pub struct StorageCleanup {
    count: i64,
}

impl StorageCleanup {
    /// Records one removal result; missing files are normal after failed
    /// creation or concurrent eviction and are not counted.
    pub fn observe(&mut self, result: std::io::Result<()>) {
        match result {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => self.count = self.count.saturating_add(1),
        }
    }

    /// Removes a directory only when empty; retained contents belong to the caller.
    pub fn empty_directory(&mut self, path: &std::path::Path) {
        match std::fs::read_dir(path) {
            Ok(mut entries) => match entries.next() {
                None => self.observe(std::fs::remove_dir(path)),
                Some(Ok(_)) => {}
                Some(Err(error)) => self.observe(Err(error)),
            },
            Err(error) => self.observe(Err(error)),
        }
    }

    /// Number of failed removals observed so far.
    pub fn failures(&self) -> i64 {
        self.count
    }

    /// Reports the batch once when anything failed, preserving the caller's
    /// normal result. `stage` names the cleanup and becomes part of the log
    /// category (`storage.cleanup_failed.<stage>`), so it must be a fixed
    /// identifier, never a path or user content.
    pub fn finish(self, stage: &str) {
        let count = u64::try_from(self.count).unwrap_or(0);
        if count > 0 {
            crate::host::diagnostics::cleanup_failed(stage, count);
        }
    }
}

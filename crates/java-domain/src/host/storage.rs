//! Best-effort cleanup bookkeeping for plugin-owned storage.
//!
//! Rust Core recorded failed cleanups as one `app.storage.cleanup_failed`
//! diagnostics event per batch. The plugin keeps the same fixed-size summary
//! and reports it on its own log channel (standard error of the backend).

/// Fixed-size cleanup summary: batch callers emit once rather than once per file.
#[derive(Debug, Default)]
pub struct StorageCleanup {
    count: i64,
    first: Option<std::io::ErrorKind>,
}

impl StorageCleanup {
    /// Records one removal result; missing files are normal after failed
    /// creation or concurrent eviction and are not counted.
    pub fn observe(&mut self, result: std::io::Result<()>) {
        match result {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                self.count = self.count.saturating_add(1);
                self.first.get_or_insert(error.kind());
            }
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

    /// Reports the batch once, preserving the caller's normal result.
    // TODO(host-api: API-22 log and progress): plugin log channel — report
    // through the host's structured plugin log instead of standard error once
    // it is available.
    pub fn finish(self, stage: &str) {
        if let Some(kind) = self.first {
            eprintln!(
                "storage cleanup failed: stage={stage} count={} error_code={kind:?}",
                self.count
            );
        }
    }
}

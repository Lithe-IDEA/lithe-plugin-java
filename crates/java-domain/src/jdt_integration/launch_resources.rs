//! Helpers for the files the JDT LS launch takes from the plugin package.
//!
//! Ported from the Swift `MacJdtLaunchResources`. The package layout itself is
//! fixed by `backend/src/launch.rs`, which resolves the launch for the host
//! (`launch.kind: "backend"`); exactly the packaged files are handed over,
//! nothing searches the host application or the machine for JDT LS, so
//! removing the plugin removes Java language support with it.

use std::fs;
use std::path::{Path, PathBuf};

/// Message shown when the package lacks a file the launch needs.
///
/// TODO(host-api: API-37 i18n): user-facing text must go through the host's
/// i18n table.
pub fn missing_resource_message(name: &str) -> String {
    format!("The Java Support plugin does not provide {name}. Reinstall the plugin.")
}

/// JAR files directly inside `directory`, sorted by name so JDT LS loads the
/// same extensions in the same order on every launch. Hidden entries,
/// directories and unreadable directories contribute nothing.
pub fn bundles_in(directory: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut bundles = entries
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            !name.starts_with('.')
                && Path::new(name.as_ref())
                    .extension()
                    .is_some_and(|ext| ext == "jar")
                && fs::metadata(entry.path()).is_ok_and(|metadata| metadata.is_file())
        })
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    bundles.sort_by(|left, right| left.file_name().cmp(&right.file_name()));
    bundles
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "lithe-java-launch-resources-{name}-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn bundles_are_sorted_and_skip_hidden_entries_and_directories() {
        let temp = TempDir::new("map");
        let debug = temp.0.join("language-server/java-debug");
        fs::create_dir_all(&debug).unwrap();
        fs::create_dir_all(debug.join("nested.jar")).unwrap();
        for file in [
            debug.join("z-debug.jar"),
            debug.join("a-debug.jar"),
            debug.join(".hidden.jar"),
            debug.join("notes.txt"),
        ] {
            fs::write(file, "").unwrap();
        }
        assert_eq!(
            bundles_in(&debug),
            vec![debug.join("a-debug.jar"), debug.join("z-debug.jar")]
        );
    }

    #[test]
    fn a_missing_bundle_directory_contributes_nothing() {
        let temp = TempDir::new("absent");
        assert!(bundles_in(&temp.0.join("language-server/java-debug")).is_empty());
    }

    #[test]
    fn missing_file_message_names_the_file() {
        assert_eq!(
            missing_resource_message("the Lombok agent"),
            "The Java Support plugin does not provide the Lombok agent. Reinstall the plugin."
        );
    }
}

//! Java argument files that keep long launch commands within the platform
//! command-line limit.
//!
//! Ported from the Swift `MacJavaLaunchArgumentPreparer`. The plan itself
//! (eligibility, option placement and quoting) is
//! [`plan_launch_command`](super::plan_launch_command); this module reads the
//! JDK feature version, writes the planned file exclusively with owner-only
//! permissions and deletes it when the launch no longer needs it.
//!
//! TODO(host-api: API-20 storage): the backend has no plugin-owned temporary
//! directory yet; callers pass the directory, and the host keeps writing
//! argument files for launches it spawns itself.

use std::collections::hash_map::RandomState;
use std::fs;
use std::hash::BuildHasher;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::{java_feature_version_from_release, plan_launch_command, LaunchCommandPlan};
use crate::protocol::{CoreError, ErrorCode};

/// Directories above the executable searched for the JDK `release` file:
/// `bin/java` reaches `Contents/Home/release` and bundle layouts above it.
const RELEASE_SEARCH_DEPTH: usize = 6;

/// Feature version of the JDK that owns `executable`, read from its
/// `release` file without running Java.
pub fn java_feature_version(executable: &Path) -> Option<u32> {
    let mut directory = executable.parent()?;
    for _ in 0..RELEASE_SEARCH_DEPTH {
        if let Ok(contents) = fs::read_to_string(directory.join("release")) {
            // The first release file that names a version decides, even when
            // the version cannot be parsed.
            if contents
                .lines()
                .any(|line| line.trim().starts_with("JAVA_VERSION="))
            {
                return java_feature_version_from_release(&contents);
            }
        }
        directory = directory.parent()?;
    }
    None
}

/// A fresh argument-file path inside `directory`.
pub fn next_argfile_path(directory: &Path) -> PathBuf {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let nonce = RandomState::new().hash_one(sequence);
    directory.join(format!(
        "launch-{}-{sequence}-{nonce:016x}.argfile",
        std::process::id()
    ))
}

/// Keeps an argument file alive until the launched process has read it.
#[derive(Debug)]
pub struct ArgumentFileLease {
    path: PathBuf,
}

impl ArgumentFileLease {
    /// Path of the argument file.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ArgumentFileLease {
    fn drop(&mut self) {
        match fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => crate::host::diagnostics::cleanup_failed("java-argfile-remove", 1),
        }
    }
}

/// Arguments to spawn, and the argument file they reference.
#[derive(Debug)]
pub struct PreparedJavaLaunch {
    /// Arguments to pass after the executable.
    pub arguments: Vec<String>,
    /// Argument file to keep until the process started; `None` for direct
    /// launches.
    pub lease: Option<ArgumentFileLease>,
}

/// Shortens `arguments` with an argument file at `argfile_path` when the
/// command would exceed the platform limit.
pub fn prepare_java_launch(
    executable: &str,
    arguments: Vec<String>,
    argfile_path: &Path,
    limit: Option<usize>,
) -> Result<PreparedJavaLaunch, CoreError> {
    let feature_version = java_feature_version(Path::new(executable));
    match plan_launch_command(executable, &arguments, argfile_path, limit, feature_version) {
        LaunchCommandPlan::Direct => Ok(PreparedJavaLaunch {
            arguments,
            lease: None,
        }),
        LaunchCommandPlan::Argfile {
            arguments,
            argfile_contents,
        } => {
            write_exclusively(argfile_path, argfile_contents.as_bytes()).map_err(|error| {
                CoreError::new(
                    ErrorCode::Unknown,
                    format!(
                        "Could not write Java launch argument file at {}: {error}",
                        argfile_path.display()
                    ),
                )
            })?;
            Ok(PreparedJavaLaunch {
                arguments,
                lease: Some(ArgumentFileLease {
                    path: argfile_path.to_path_buf(),
                }),
            })
        }
    }
}

/// Creates `path` (never replacing a file), readable only by its owner.
fn write_exclusively(path: &Path, contents: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    if let Err(error) = file.write_all(contents) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("lithe-java-argfile-{name}-{}", std::process::id()));
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

    fn jdk(root: &Path, release: &str) -> PathBuf {
        let home = root.join("jdk/Contents/Home");
        fs::create_dir_all(home.join("bin")).unwrap();
        fs::write(home.join("release"), release).unwrap();
        home.join("bin/java")
    }

    #[test]
    fn the_feature_version_comes_from_the_bundle_release_file() {
        let temp = TempDir::new("version");
        let java = jdk(
            &temp.0,
            "IMPLEMENTOR=\"Eclipse\"\nJAVA_VERSION=\"21.0.2\"\n",
        );
        assert_eq!(java_feature_version(&java), Some(21));
        fs::write(
            temp.0.join("jdk/Contents/Home/release"),
            "JAVA_VERSION=\"1.8.0_442\"\n",
        )
        .unwrap();
        assert_eq!(java_feature_version(&java), Some(8));
        fs::remove_file(temp.0.join("jdk/Contents/Home/release")).unwrap();
        assert_eq!(java_feature_version(&java), None);
    }

    #[test]
    fn long_commands_write_an_owner_only_argfile_that_the_lease_removes() {
        let temp = TempDir::new("write");
        let java = jdk(&temp.0, "JAVA_VERSION=\"21.0.2\"\n");
        let class_path = (0..400)
            .map(|index| format!("/repository/dependency-{index}.jar"))
            .collect::<Vec<_>>()
            .join(":");
        let arguments = vec!["-cp".to_string(), class_path, "app.Main".into()];
        let path = next_argfile_path(&temp.0.join("lithe-run"));
        let prepared = prepare_java_launch(
            java.to_str().unwrap(),
            arguments.clone(),
            &path,
            Some(1_000),
        )
        .unwrap();
        let lease = prepared.lease.expect("argument file");
        assert_eq!(lease.path(), path);
        assert!(prepared.arguments.contains(&format!("@{}", path.display())));
        assert!(fs::read_to_string(&path)
            .unwrap()
            .contains("dependency-399.jar"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        // An existing file is never replaced.
        let error =
            prepare_java_launch(java.to_str().unwrap(), arguments, &path, Some(1_000)).unwrap_err();
        assert!(error
            .message
            .starts_with("Could not write Java launch argument file at "));
        drop(lease);
        assert!(!path.exists());
    }

    #[test]
    fn short_commands_and_old_jdks_launch_directly() {
        let temp = TempDir::new("direct");
        let java = jdk(&temp.0, "JAVA_VERSION=\"1.8.0_442\"\n");
        let path = temp.0.join("unused.argfile");
        let long = vec!["-cp".to_string(), "x".repeat(5_000), "app.Main".into()];
        let prepared =
            prepare_java_launch(java.to_str().unwrap(), long.clone(), &path, Some(1_000)).unwrap();
        assert_eq!(prepared.arguments, long);
        assert!(prepared.lease.is_none());
        assert!(!path.exists());
    }

    #[test]
    fn argfile_paths_are_unique() {
        let directory = Path::new("/tmp/lithe-run");
        assert_ne!(next_argfile_path(directory), next_argfile_path(directory));
    }
}

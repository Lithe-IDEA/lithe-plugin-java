//! JDT LS state in the host cache: fingerprints, index reset and retention.
//!
//! Ported from the Swift `MacJdtWorkspaceState`. The rules only inspect
//! immediate build-system inputs and the plugin's own JDT LS installation;
//! key derivation and the retention policy are the shared ones in
//! [`crate::lsp`].
//!
//! Layout below the cache directory the host gives the language server:
//! `jdtls/<workspace key>/` holds each workspace's `-data` directory and
//! `jdtls-configuration/` the writable Equinox configuration areas.
//!
//! TODO(host-api: API-20 storage): the cache directory comes from the host.
//! Until the host exposes a plugin-owned cache directory to the backend,
//! callers pass the host's language-server cache directory explicitly.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::host::storage::StorageCleanup;
use crate::lsp::{
    jdt_cache_retention, jdt_workspace_fingerprint, workspace_key, JdtBuildFileObservation,
    JdtCacheEntry, JdtCacheRetentionRequest, JdtWorkspaceFingerprintRequest,
};
use crate::protocol::{CoreError, ErrorCode};

/// Root build descriptors whose presence selects a JDT LS state directory.
pub const BUILD_FILE_NAMES: &[&str] = &["pom.xml", "build.gradle", "build.gradle.kts"];
/// Cache directories that hold JDT LS state: per-workspace indexes and
/// Equinox configuration areas.
pub const OWNED_CACHE_DIRECTORY_NAMES: &[&str] = &["jdtls", "jdtls-configuration"];
/// Marker file whose modification time records the last use of an index.
pub const LAST_USED_MARKER_NAME: &str = ".lithe-last-used";

const MAVEN_DESCRIPTOR_NAME: &str = "pom.xml";
const MANIFEST_NAME: &str = "manifest.json";
const CORE_PLUGIN_PREFIX: &str = "org.eclipse.jdt.ls.core_";

/// Build inputs observed at a workspace root, without recursion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceObservation {
    /// Root build descriptors that exist.
    pub build_files: Vec<JdtBuildFileObservation>,
    /// Direct child directories that contain a regular `pom.xml`.
    pub direct_maven_modules: Vec<String>,
}

/// Observes the root build descriptors and direct Maven modules.
///
/// Only presence counts: JDT LS detects edits to descriptors itself.
pub fn observe_workspace(workspace_root: &Path) -> Result<WorkspaceObservation, CoreError> {
    let mut build_files = Vec::new();
    for name in BUILD_FILE_NAMES {
        match fs::symlink_metadata(workspace_root.join(name)) {
            Ok(_) => build_files.push(JdtBuildFileObservation {
                path: (*name).to_string(),
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error("Could not inspect the Java workspace.", &error)),
        }
    }
    let mut direct_maven_modules = Vec::new();
    let entries = fs::read_dir(workspace_root)
        .map_err(|error| io_error("Could not list the Java workspace.", &error))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| io_error("Could not list the Java workspace.", &error))?;
        let path = entry.path();
        // Follow links like Foundation's `isDirectoryKey` on the entry URL.
        if !path.is_dir() {
            continue;
        }
        match fs::metadata(path.join(MAVEN_DESCRIPTOR_NAME)) {
            Ok(metadata) if metadata.is_file() => {
                direct_maven_modules.push(entry.file_name().to_string_lossy().into_owned());
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error("Could not inspect a Maven module.", &error)),
        }
    }
    direct_maven_modules.sort();
    Ok(WorkspaceObservation {
        build_files,
        direct_maven_modules,
    })
}

#[derive(Deserialize)]
struct JdtManifest {
    version: String,
}

/// Version of the JDT LS installation that `launcher` belongs to.
///
/// The launcher's directory and its parent are searched for `manifest.json`;
/// without one, the core plug-in's file name supplies the first three
/// version components.
pub fn language_server_version(launcher: &Path) -> Result<String, CoreError> {
    let bin = launcher.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut roots = vec![bin.clone()];
    if let Some(parent) = bin.parent() {
        roots.push(parent.to_path_buf());
    }
    roots.dedup();
    for root in roots {
        let manifest_path = root.join(MANIFEST_NAME);
        match fs::read(&manifest_path) {
            Ok(data) => {
                let version = serde_json::from_slice::<JdtManifest>(&data)
                    .map(|manifest| manifest.version.trim().to_string())
                    .ok()
                    .filter(|version| !version.is_empty());
                return version.ok_or_else(|| {
                    CoreError::new(
                        ErrorCode::ParseFailed,
                        format!(
                            "The packaged JDT LS manifest is invalid at {}.",
                            manifest_path.display()
                        ),
                    )
                });
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error("Could not read the JDT LS manifest.", &error)),
        }
        let names = match fs::read_dir(root.join("plugins")) {
            Ok(entries) => entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(io_error("Could not list the JDT LS plug-ins.", &error)),
        };
        let mut names = names;
        names.sort();
        if let Some(suffix) = names.iter().find_map(|name| {
            name.strip_prefix(CORE_PLUGIN_PREFIX)
                .and_then(|rest| rest.strip_suffix(".jar"))
        }) {
            let components = suffix.split('.').collect::<Vec<_>>();
            if components.len() >= 3 {
                return Ok(components[..3].join("."));
            }
            return Ok(suffix.to_string());
        }
    }
    Err(CoreError::new(
        ErrorCode::RuntimeMissing,
        "The packaged JDT LS version could not be determined.",
    ))
}

/// Version and location of a JDT LS installation.
///
/// JDT LS persists absolute JRE and source paths inside its workspace, so a
/// relocated plugin package must select a fresh state directory.
pub fn language_server_cache_identity(launcher: &Path) -> Result<String, CoreError> {
    let version = language_server_version(launcher)?;
    Ok(format!(
        "{version}|installation={}",
        crate::testing::workflow::normalize(launcher).display()
    ))
}

/// Request for [`jdt_workspace_fingerprint`] describing one workspace.
pub fn fingerprint_request(
    workspace_root: &Path,
    launcher: &Path,
) -> Result<JdtWorkspaceFingerprintRequest, CoreError> {
    let observation = observe_workspace(workspace_root)?;
    Ok(JdtWorkspaceFingerprintRequest {
        build_files: observation.build_files,
        direct_maven_modules: observation.direct_maven_modules,
        gradle_wrapper_distribution: None,
        jdtls_version: language_server_cache_identity(launcher)?,
    })
}

/// Fingerprint mixed into the workspace key of `workspace_root`.
pub fn fingerprint(workspace_root: &Path, launcher: &Path) -> Result<String, CoreError> {
    Ok(
        jdt_workspace_fingerprint(fingerprint_request(workspace_root, launcher)?)?
            .workspace_fingerprint,
    )
}

/// Whether `key` is a lowercase SHA-256 workspace key.
pub fn is_workspace_key(key: &str) -> bool {
    key.len() == 64
        && key
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn checked_key(workspace_root: &Path, fingerprint: Option<&str>) -> Result<String, CoreError> {
    let key = workspace_key(workspace_root, fingerprint);
    if is_workspace_key(&key) {
        Ok(key)
    } else {
        Err(CoreError::new(
            ErrorCode::InvalidRequest,
            "The Java workspace key is invalid.",
        ))
    }
}

/// Removes the index of one workspace and fingerprint.
///
/// Other fingerprints of the same workspace stay, so switching back to them
/// reuses their index.
pub fn clear_index(
    cache_directory: &Path,
    workspace_root: &Path,
    fingerprint: Option<&str>,
) -> Result<(), CoreError> {
    let key = checked_key(workspace_root, fingerprint)?;
    let target = cache_directory.join("jdtls").join(key);
    match fs::remove_dir_all(&target) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error("Could not remove the Java index.", &error)),
    }
}

/// Outcome of [`prune_expired_caches`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheCleanupResult {
    /// Retention period of the shared policy.
    pub retention_days: u64,
    /// Workspace keys whose directories were removed.
    pub removed_workspace_keys: Vec<String>,
}

/// Removes inactive indexes the shared retention policy selects.
///
/// The active workspace's directory is marked as used at `now` first. Hidden
/// entries, symbolic links and names that are not workspace keys are never
/// touched.
pub fn prune_expired_caches(
    cache_directory: &Path,
    workspace_root: &Path,
    fingerprint: Option<&str>,
    now: SystemTime,
) -> Result<CacheCleanupResult, CoreError> {
    let active_key = checked_key(workspace_root, fingerprint)?;
    let now_seconds = unix_seconds(now);
    let jdtls = cache_directory.join("jdtls");
    let candidates = match fs::read_dir(&jdtls) {
        Ok(entries) => entries
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| io_error("Could not list the Java indexes.", &error))?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(io_error("Could not list the Java indexes.", &error)),
    };
    let mut entries = Vec::new();
    for candidate in candidates {
        let key = candidate.file_name().to_string_lossy().into_owned();
        if key.starts_with('.') || !is_workspace_key(&key) {
            continue;
        }
        let path = candidate.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| io_error("Could not inspect a Java index.", &error))?;
        if !metadata.is_dir() {
            continue;
        }
        let directory_modified = metadata.modified().ok().map(unix_seconds);
        let marker_modified = if key == active_key {
            write_marker(&path, now_seconds)?;
            Some(now_seconds)
        } else {
            match fs::metadata(path.join(LAST_USED_MARKER_NAME)) {
                Ok(marker) => marker.modified().ok().map(unix_seconds),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(io_error("Could not inspect a Java index.", &error)),
            }
        };
        entries.push(JdtCacheEntry {
            workspace_key: key,
            last_modified_unix_seconds: directory_modified.max(marker_modified).unwrap_or(0),
        });
    }
    let observed = entries
        .iter()
        .map(|entry| entry.workspace_key.clone())
        .collect::<Vec<_>>();
    let plan = jdt_cache_retention(JdtCacheRetentionRequest {
        now_unix_seconds: now_seconds,
        active_workspace_key: Some(active_key.clone()),
        entries,
    })?;
    let mut removed = Vec::new();
    for key in plan.expired_workspace_keys {
        if key == active_key || !observed.contains(&key) || !is_workspace_key(&key) {
            return Err(CoreError::new(
                ErrorCode::InvalidRequest,
                "The Java cache-retention policy selected a key that was not observed.",
            ));
        }
        let target = jdtls.join(&key);
        let metadata = fs::symlink_metadata(&target)
            .map_err(|error| io_error("Could not inspect a Java index.", &error))?;
        if !metadata.is_dir() {
            return Err(CoreError::new(
                ErrorCode::InvalidRequest,
                "A Java cache-retention target is not a removable directory.",
            ));
        }
        fs::remove_dir_all(&target)
            .map_err(|error| io_error("Could not remove a Java index.", &error))?;
        removed.push(key);
    }
    Ok(CacheCleanupResult {
        retention_days: plan.retention_days,
        removed_workspace_keys: removed,
    })
}

/// Removes every index and Equinox area, for when the plugin is gone.
///
/// Best effort: it only frees disk space. Missing directories are normal.
pub fn remove_all_state(cache_directory: &Path) -> StorageCleanup {
    let mut cleanup = StorageCleanup::default();
    for name in OWNED_CACHE_DIRECTORY_NAMES {
        cleanup.observe(fs::remove_dir_all(cache_directory.join(name)));
    }
    cleanup
}

fn write_marker(directory: &Path, now_seconds: u64) -> Result<(), CoreError> {
    let marker = directory.join(LAST_USED_MARKER_NAME);
    let staging: PathBuf = directory.join(format!("{LAST_USED_MARKER_NAME}.tmp"));
    fs::write(&staging, now_seconds.to_string())
        .and_then(|()| fs::rename(&staging, &marker))
        .map_err(|error| io_error("Could not mark the Java index as used.", &error))
}

fn unix_seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn io_error(message: &str, error: &io::Error) -> CoreError {
    let code = if error.kind() == io::ErrorKind::PermissionDenied {
        ErrorCode::PermissionDenied
    } else {
        ErrorCode::Unknown
    };
    CoreError::new(code, message).with_details(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::Duration;

    struct Fixture {
        root: PathBuf,
        workspace: PathBuf,
        cache: PathBuf,
        launcher: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "lithe-java-jdt-state-{name}-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            let workspace = root.join("workspace");
            let cache = root.join("cache");
            let bin = root.join("jdtls").join("bin");
            for directory in [&workspace, &cache, &bin] {
                fs::create_dir_all(directory).unwrap();
            }
            fs::write(root.join("jdtls/manifest.json"), r#"{"version":"7.6.5"}"#).unwrap();
            let launcher = bin.join("jdtls");
            fs::write(&launcher, "#!/bin/zsh\n").unwrap();
            Self {
                root,
                workspace,
                cache,
                launcher,
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn set_modified(path: &Path, seconds: u64) {
        filetime::set_file_mtime(
            path,
            filetime::FileTime::from_unix_time(seconds.try_into().unwrap(), 0),
        )
        .unwrap();
    }

    #[test]
    fn identity_includes_the_version_and_installation_path() {
        let fixture = Fixture::new("identity");
        let request = fingerprint_request(&fixture.workspace, &fixture.launcher).unwrap();
        assert!(request.build_files.is_empty());
        assert!(request.direct_maven_modules.is_empty());
        assert_eq!(
            request.jdtls_version,
            format!("7.6.5|installation={}", fixture.launcher.display())
        );
        assert!(!fingerprint(&fixture.workspace, &fixture.launcher)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn observation_counts_root_descriptors_and_direct_maven_modules_only() {
        let fixture = Fixture::new("observation");
        let pom = fixture.workspace.join("pom.xml");
        fs::write(&pom, "a").unwrap();
        let before_edit = observe_workspace(&fixture.workspace).unwrap();
        assert_eq!(
            before_edit.build_files,
            vec![JdtBuildFileObservation {
                path: "pom.xml".into()
            }]
        );
        // Editing a descriptor keeps the JDT LS state: JDT LS updates the
        // changed projects itself instead of reimporting the workspace.
        fs::write(&pom, "a larger project descriptor").unwrap();
        set_modified(&pom, 2_000_000_000);
        assert_eq!(
            observe_workspace(&fixture.workspace).unwrap().build_files,
            before_edit.build_files
        );
        for module in ["zeta", "alpha"] {
            fs::create_dir_all(fixture.workspace.join(module)).unwrap();
            fs::write(fixture.workspace.join(module).join("pom.xml"), "<project/>").unwrap();
        }
        fs::create_dir_all(fixture.workspace.join("gradle-only")).unwrap();
        fs::write(
            fixture.workspace.join("gradle-only/build.gradle"),
            "plugins {}",
        )
        .unwrap();
        assert_eq!(
            observe_workspace(&fixture.workspace)
                .unwrap()
                .direct_maven_modules,
            vec!["alpha", "zeta"]
        );
    }

    #[test]
    fn the_core_plugin_name_supplies_the_version_without_a_manifest() {
        let fixture = Fixture::new("plugin-version");
        fs::remove_file(fixture.root.join("jdtls/manifest.json")).unwrap();
        fs::create_dir_all(fixture.root.join("jdtls/plugins")).unwrap();
        fs::write(
            fixture
                .root
                .join("jdtls/plugins/org.eclipse.jdt.ls.core_1.52.0.202607301513.jar"),
            "",
        )
        .unwrap();
        assert_eq!(
            language_server_version(&fixture.launcher).unwrap(),
            "1.52.0"
        );
        fs::remove_dir_all(fixture.root.join("jdtls/plugins")).unwrap();
        assert_eq!(
            language_server_version(&fixture.launcher).unwrap_err().code,
            ErrorCode::RuntimeMissing
        );
    }

    #[test]
    fn clear_index_removes_only_the_resolved_workspace_key() {
        let fixture = Fixture::new("clear");
        let current = workspace_key(&fixture.workspace, Some("current-fingerprint"));
        let sibling = "b".repeat(64);
        for key in [&current, &sibling] {
            let directory = fixture.cache.join("jdtls").join(key);
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("index.bin"), key.as_bytes()).unwrap();
        }
        clear_index(
            &fixture.cache,
            &fixture.workspace,
            Some("current-fingerprint"),
        )
        .unwrap();
        assert!(!fixture.cache.join("jdtls").join(&current).exists());
        assert!(fixture.cache.join("jdtls").join(&sibling).exists());
        clear_index(
            &fixture.cache,
            &fixture.workspace,
            Some("current-fingerprint"),
        )
        .unwrap();
    }

    #[test]
    fn cache_cleanup_removes_only_expired_inactive_directories() {
        let fixture = Fixture::new("prune");
        let jdtls = fixture.cache.join("jdtls");
        let active = workspace_key(&fixture.workspace, Some("active-fingerprint"));
        let expired = "a".repeat(64);
        let recent = "b".repeat(64);
        let linked = "d".repeat(64);
        for key in [&expired, &recent, &active] {
            fs::create_dir_all(jdtls.join(key)).unwrap();
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(jdtls.join(&recent), jdtls.join(&linked)).unwrap();
        // A local NTFS junction exercises the alias boundary without requiring
        // Administrator or Developer Mode merely to run this test.
        #[cfg(windows)]
        junction::create(jdtls.join(&recent), jdtls.join(&linked)).unwrap();
        fs::create_dir_all(jdtls.join("not-a-cache-key")).unwrap();
        set_modified(&jdtls.join(&expired), 1_000_000);
        set_modified(&jdtls.join(&recent), 3_999_999);
        set_modified(&jdtls.join(&active), 1);

        let result = prune_expired_caches(
            &fixture.cache,
            &fixture.workspace,
            Some("active-fingerprint"),
            UNIX_EPOCH + Duration::from_secs(4_000_000),
        )
        .unwrap();

        assert_eq!(
            result,
            CacheCleanupResult {
                retention_days: 30,
                removed_workspace_keys: vec![expired.clone()],
            }
        );
        assert!(!jdtls.join(&expired).exists());
        for key in [
            recent.as_str(),
            active.as_str(),
            linked.as_str(),
            "not-a-cache-key",
        ] {
            assert!(fs::symlink_metadata(jdtls.join(key)).is_ok(), "{key}");
        }
        assert_eq!(
            fs::read_to_string(jdtls.join(&active).join(LAST_USED_MARKER_NAME)).unwrap(),
            "4000000"
        );
    }

    #[test]
    fn removing_all_state_keeps_other_servers_caches() {
        let fixture = Fixture::new("remove-all");
        let key = "a".repeat(64);
        let index = fixture.cache.join("jdtls").join(&key);
        let equinox = fixture
            .cache
            .join("jdtls-configuration")
            .join(&key)
            .join("configuration");
        let unrelated = fixture.cache.join("other-server");
        for directory in [&index, &equinox, &unrelated] {
            fs::create_dir_all(directory).unwrap();
            fs::write(directory.join("marker"), "state").unwrap();
        }
        assert_eq!(remove_all_state(&fixture.cache).failures(), 0);
        assert!(!index.exists());
        assert!(!equinox.exists());
        assert!(unrelated.exists());
        assert_eq!(remove_all_state(&fixture.cache).failures(), 0);
    }
}

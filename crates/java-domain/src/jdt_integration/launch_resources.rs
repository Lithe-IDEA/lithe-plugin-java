//! Files the JDT LS launch takes from the plugin declaration.
//!
//! Ported from the Swift `MacJdtLaunchResources`. Exactly the declared files
//! are handed over; nothing searches the host application or the machine for
//! JDT LS, so removing the plugin removes Java language support with it.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::protocol::{CoreError, ErrorCode};

/// Declaration key of the writable-copy source for the Equinox configuration.
pub const CONFIGURATION_RESOURCE: &str = "configuration";
/// Declaration key of the Lombok Java agent.
pub const LOMBOK_AGENT_RESOURCE: &str = "lombokAgent";
/// Declaration key of the directories whose JARs extend JDT LS.
pub const EXTENSION_BUNDLE_DIRECTORIES_RESOURCE: &str = "extensionBundleDirectories";
/// Declaration key of the Java Test runner JAR.
pub const TEST_RUNNER_RESOURCE: &str = "testRunner";

/// A resolved `hostIntegration` declaration: package-relative paths already
/// joined to the installed package directory.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeclaredHostIntegration {
    /// The language server's `launch.entry`, the Equinox launcher JAR.
    pub entry: Option<PathBuf>,
    /// `hostIntegration.resources`; single values are one-element lists.
    pub resources: BTreeMap<String, Vec<PathBuf>>,
}

impl DeclaredHostIntegration {
    /// First path declared under `name`.
    pub fn resource(&self, name: &str) -> Option<&Path> {
        self.resources
            .get(name)
            .and_then(|paths| paths.first())
            .map(PathBuf::as_path)
    }
}

/// Files of one JDT LS launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JdtLaunchResources {
    /// Equinox launcher JAR.
    pub launcher_jar: PathBuf,
    /// Packaged configuration directory. It is passed as is: the launch gives
    /// Equinox a writable copy in the host cache and never writes into the
    /// package.
    pub configuration_directory: PathBuf,
    /// Lombok Java agent.
    pub lombok_agent: PathBuf,
    /// JDT LS extension bundles in load order.
    pub extension_bundles: Vec<PathBuf>,
    /// Java Test runner JAR, when the package ships one.
    pub test_runner: Option<PathBuf>,
}

/// Maps a declaration to launch files, naming the first missing one.
pub fn launch_resources(
    integration: &DeclaredHostIntegration,
) -> Result<JdtLaunchResources, CoreError> {
    let launcher_jar = integration
        .entry
        .clone()
        .ok_or_else(|| missing_resource("the JDT LS launcher"))?;
    let configuration_directory = integration
        .resource(CONFIGURATION_RESOURCE)
        .ok_or_else(|| missing_resource("the JDT LS configuration"))?
        .to_path_buf();
    let lombok_agent = integration
        .resource(LOMBOK_AGENT_RESOURCE)
        .ok_or_else(|| missing_resource("the Lombok agent"))?
        .to_path_buf();
    let extension_bundles = integration
        .resources
        .get(EXTENSION_BUNDLE_DIRECTORIES_RESOURCE)
        .map(|directories| {
            directories
                .iter()
                .flat_map(|directory| bundles_in(directory))
                .collect()
        })
        .unwrap_or_default();
    Ok(JdtLaunchResources {
        launcher_jar,
        configuration_directory,
        lombok_agent,
        extension_bundles,
        test_runner: integration
            .resource(TEST_RUNNER_RESOURCE)
            .map(Path::to_path_buf),
    })
}

/// Message shown when the package lacks a declared file.
///
/// TODO(host-api: API-37 i18n): user-facing text must go through the host's
/// i18n table.
pub fn missing_resource_message(name: &str) -> String {
    format!("The Java Support plugin does not provide {name}. Reinstall the plugin.")
}

fn missing_resource(name: &str) -> CoreError {
    CoreError::new(ErrorCode::RuntimeMissing, missing_resource_message(name))
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

    fn declaration(root: &Path) -> DeclaredHostIntegration {
        let mut resources = BTreeMap::new();
        resources.insert(
            CONFIGURATION_RESOURCE.into(),
            vec![root.join("language-server/configuration/arm64")],
        );
        resources.insert(
            LOMBOK_AGENT_RESOURCE.into(),
            vec![root.join("language-server/lombok/lombok.jar")],
        );
        resources.insert(
            EXTENSION_BUNDLE_DIRECTORIES_RESOURCE.into(),
            vec![
                root.join("language-server/java-debug"),
                root.join("language-server/java-test/extensions"),
            ],
        );
        DeclaredHostIntegration {
            entry: Some(root.join("language-server/plugins/launcher.jar")),
            resources,
        }
    }

    #[test]
    fn declared_files_map_to_the_launch_with_sorted_extension_bundles() {
        let temp = TempDir::new("map");
        let debug = temp.0.join("language-server/java-debug");
        let test = temp.0.join("language-server/java-test/extensions");
        fs::create_dir_all(&debug).unwrap();
        fs::create_dir_all(test.join("nested.jar")).unwrap();
        for file in [
            debug.join("z-debug.jar"),
            debug.join("a-debug.jar"),
            debug.join(".hidden.jar"),
            debug.join("notes.txt"),
            test.join("test.jar"),
        ] {
            fs::write(file, "").unwrap();
        }
        let resources = launch_resources(&declaration(&temp.0)).unwrap();
        assert_eq!(
            resources.launcher_jar,
            temp.0.join("language-server/plugins/launcher.jar")
        );
        assert_eq!(
            resources.extension_bundles,
            vec![
                debug.join("a-debug.jar"),
                debug.join("z-debug.jar"),
                test.join("test.jar")
            ]
        );
        assert_eq!(resources.test_runner, None);
    }

    #[test]
    fn missing_declarations_name_the_missing_file() {
        let temp = TempDir::new("missing");
        let mut declared = declaration(&temp.0);
        declared.entry = None;
        assert_eq!(
            launch_resources(&declared).unwrap_err().message,
            "The Java Support plugin does not provide the JDT LS launcher. Reinstall the plugin."
        );
        let mut declared = declaration(&temp.0);
        declared.resources.remove(LOMBOK_AGENT_RESOURCE);
        assert_eq!(
            launch_resources(&declared).unwrap_err().message,
            "The Java Support plugin does not provide the Lombok agent. Reinstall the plugin."
        );
        let mut declared = declaration(&temp.0);
        declared.resources.remove(CONFIGURATION_RESOURCE);
        assert_eq!(
            launch_resources(&declared).unwrap_err().message,
            "The Java Support plugin does not provide the JDT LS configuration. Reinstall the plugin."
        );
    }

    #[test]
    fn a_missing_bundle_directory_contributes_nothing() {
        let temp = TempDir::new("absent");
        let mut declared = declaration(&temp.0);
        declared
            .resources
            .insert(TEST_RUNNER_RESOURCE.into(), vec![temp.0.join("runner.jar")]);
        let resources = launch_resources(&declared).unwrap();
        assert!(resources.extension_bundles.is_empty());
        assert_eq!(resources.test_runner, Some(temp.0.join("runner.jar")));
    }
}

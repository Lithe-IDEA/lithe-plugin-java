//! The JDK that JDT LS runs on, and the lifetime of cached JDT LS state.
//!
//! Ported from the JDT parts of the Swift `ProjectRuntimeService` and
//! `MacJdtHostIntegration`. JDT LS runs only on the runtime the plugin ships;
//! project JDKs and user environment variables never influence it.

use std::path::{Path, PathBuf};

/// Oldest Java feature release that runs the packaged JDT LS.
pub const MINIMUM_JDTLS_JAVA_MAJOR: u32 = 21;

/// Major version of a `java -version` string.
///
/// Legacy `1.x` versions report their second component: `1.8.0_442` is 8.
pub fn java_major_version(version: &str) -> Option<u32> {
    let mut components = version
        .split(|character: char| !character.is_ascii_digit())
        .filter(|component| !component.is_empty())
        .map(str::parse::<u32>);
    match components.next()? {
        Ok(1) => components.next()?.ok(),
        Ok(major) => Some(major),
        Err(_) => None,
    }
}

/// Whether a runtime with `version` can run JDT LS.
pub fn supports_jdtls(version: &str) -> bool {
    java_major_version(version).is_some_and(|major| major >= MINIMUM_JDTLS_JAVA_MAJOR)
}

/// JDK home of a packaged `<home>/bin/java` executable.
pub fn runtime_home(packaged_java: &Path) -> Option<PathBuf> {
    packaged_java.parent()?.parent().map(Path::to_path_buf)
}

/// A JDK found at a home directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbedRuntime {
    /// Version reported by the runtime.
    pub version: String,
    /// Home directory the runtime reported.
    pub home: PathBuf,
}

/// Outcome of preparing the JDT LS runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimePreparation {
    /// JDT LS can start with this executable.
    Ready {
        /// `<home>/bin/java` of the packaged runtime.
        executable: PathBuf,
    },
    /// No installed plugin provides the runtime. Not worth caching: installing
    /// or restoring the plugin changes the answer.
    PluginMissing,
    /// The packaged runtime cannot run JDT LS.
    Failed {
        /// Explanation for the user.
        message: String,
    },
}

impl RuntimePreparation {
    /// Explanation for the user when JDT LS cannot start.
    ///
    /// TODO(host-api: API-37 i18n): user-facing text must go through the host's
    /// i18n table.
    pub fn failure_message(&self) -> Option<String> {
        match self {
            Self::Ready { .. } => None,
            Self::PluginMissing => Some(
                "Java language support is provided by the Java Support plugin, which is not installed. Install or restore it in Settings > Plugins."
                    .into(),
            ),
            Self::Failed { message } => Some(message.clone()),
        }
    }
}

/// Validates the packaged runtime at `home`.
///
/// `probe` reports the runtime installed at a home, and `is_valid_home`
/// whether a reported home contains a usable `bin/java`.
///
/// TODO(host-api: API-23 toolchains and process): probing runs `java -version`;
/// the backend needs the host's executable-resolution probe or a
/// `process.spawn` permission for it.
pub fn prepare_runtime(
    home: Option<&Path>,
    probe: impl FnOnce(&Path) -> Option<ProbedRuntime>,
    is_valid_home: impl FnOnce(&Path) -> bool,
) -> RuntimePreparation {
    let Some(home) = home else {
        return RuntimePreparation::PluginMissing;
    };
    let Some(runtime) = probe(home) else {
        return RuntimePreparation::Failed {
            message:
                "The Java Support plugin's runtime is missing or invalid. Reinstall the plugin."
                    .into(),
        };
    };
    if !supports_jdtls(&runtime.version) || !is_valid_home(&runtime.home) {
        return RuntimePreparation::Failed {
            message: format!(
                "The Java Support plugin's runtime must be Java {MINIMUM_JDTLS_JAVA_MAJOR} or newer; found {}. Reinstall the plugin.",
                runtime.version
            ),
        };
    }
    RuntimePreparation::Ready {
        executable: runtime.home.join("bin/java"),
    }
}

/// An installed plugin as the cache-ownership rule sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstalledPlugin {
    /// The installation was removed; disabled or rejected packages are not.
    pub removed: bool,
    /// One of its language servers declares the `jdt` host integration.
    pub declares_jdt_integration: bool,
}

/// Whether the JDT LS state in the host cache may be discarded.
///
/// The state belongs to the plugin, but lives outside its own state
/// directory. It is discarded once no installed plugin declares the
/// integration. A package that is disabled or rejected at startup is not a
/// removal, and a failed scan (`None`) never deletes anything.
pub fn should_remove_state(installed: Option<&[InstalledPlugin]>) -> bool {
    installed.is_some_and(|plugins| {
        !plugins
            .iter()
            .any(|plugin| !plugin.removed && plugin.declares_jdt_integration)
    })
}

/// Message when the language server is asked to start before the runtime
/// was prepared.
pub const RUNTIME_NOT_PREPARED_MESSAGE: &str = "The Java Support plugin's runtime is not prepared.";
/// Message when the plugin does not declare its JDT LS files.
pub const UNDECLARED_FILES_MESSAGE: &str =
    "The Java Support plugin does not declare its JDT LS files. Reinstall the plugin.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn major_versions_follow_the_java_version_scheme() {
        assert_eq!(java_major_version("17.0.18"), Some(17));
        assert_eq!(java_major_version("21-ea"), Some(21));
        assert_eq!(java_major_version("1.8.0_442"), Some(8));
        assert_eq!(java_major_version("unknown"), None);
        assert!(supports_jdtls("21.0.2"));
        assert!(supports_jdtls("25"));
        assert!(!supports_jdtls("17.0.18"));
        assert!(!supports_jdtls("1.8.0_442"));
    }

    #[test]
    fn the_runtime_home_is_two_levels_above_java() {
        assert_eq!(
            runtime_home(Path::new("/p/runtime/arm64/bin/java")),
            Some(PathBuf::from("/p/runtime/arm64"))
        );
    }

    #[test]
    fn preparation_reports_why_the_runtime_cannot_run_jdtls() {
        let home = Path::new("/p/runtime/arm64");
        assert_eq!(
            prepare_runtime(None, |_| unreachable!(), |_| true),
            RuntimePreparation::PluginMissing
        );
        assert_eq!(
            prepare_runtime(Some(home), |_| None, |_| true).failure_message(),
            Some(
                "The Java Support plugin's runtime is missing or invalid. Reinstall the plugin."
                    .into()
            )
        );
        let old = || {
            Some(ProbedRuntime {
                version: "17.0.18".into(),
                home: home.to_path_buf(),
            })
        };
        assert_eq!(
            prepare_runtime(Some(home), |_| old(), |_| true).failure_message(),
            Some("The Java Support plugin's runtime must be Java 21 or newer; found 17.0.18. Reinstall the plugin.".into())
        );
        let current = |_: &Path| {
            Some(ProbedRuntime {
                version: "21.0.2".into(),
                home: home.to_path_buf(),
            })
        };
        assert!(matches!(
            prepare_runtime(Some(home), current, |_| false),
            RuntimePreparation::Failed { .. }
        ));
        assert_eq!(
            prepare_runtime(Some(home), current, |_| true),
            RuntimePreparation::Ready {
                executable: home.join("bin/java")
            }
        );
    }

    #[test]
    fn state_is_removed_only_when_no_installed_plugin_declares_the_integration() {
        let java = InstalledPlugin {
            removed: false,
            declares_jdt_integration: true,
        };
        let removed_java = InstalledPlugin {
            removed: true,
            ..java
        };
        let other = InstalledPlugin {
            removed: false,
            declares_jdt_integration: false,
        };
        assert!(should_remove_state(Some(&[removed_java, other])));
        assert!(should_remove_state(Some(&[])));
        // Installed, disabled or rejected at startup: the plugin may return.
        assert!(!should_remove_state(Some(&[java])));
        assert!(!should_remove_state(Some(&[removed_java, java])));
        // A failed scan is not a removal.
        assert!(!should_remove_state(None));
    }
}

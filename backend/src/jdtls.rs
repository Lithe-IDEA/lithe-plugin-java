//! Finds the Eclipse JDT Language Server installed on the user's machine.
//!
//! The plugin package does not ship JDT LS or a Java runtime for it, the same
//! way the Go plugin uses the user's own `gopls`: the user installs JDT LS
//! with a package manager or an editor, and the launch reads its files from
//! there. The backend only reads that installation. Equinox gets a writable
//! copy of the installation's `config.ini` under the host's state directory
//! (`prepare_jdt_configuration_area`), so neither the backend nor JDT LS
//! writes into the user's installation.
//!
//! # Where the launch looks
//!
//! [`locate`] takes the first installation of these, in order:
//!
//! 1. the `languageServerHome` setting. A folder set there is the user's
//!    explicit choice, so an unusable one fails the launch instead of falling
//!    back to another installation;
//! 2. the `JDTLS_HOME` environment variable;
//! 3. a `jdtls` launcher on `PATH`, resolved through its symbolic links to the
//!    installation it belongs to;
//! 4. the folders package managers install JDT LS into (Homebrew, Linuxbrew,
//!    mason.nvim, Scoop);
//! 5. the language server inside an installed VS Code Java extension
//!    (`redhat.java`), newest version first.
//!
//! A folder holds an installation when it, or its `libexec/` (Homebrew keeps
//! the files there), contains exactly one Equinox launcher JAR in `plugins/`
//! and the `config_<platform>/config.ini` of the host platform. Nothing else
//! is checked: JDT LS reports a version it cannot run at `initialize`.

use lithe_plugin_sdk::PluginError;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

/// No JDT LS installation was found on the machine.
pub const LAUNCH_SERVER_MISSING: &str = "launch.server_missing";
/// The folder chosen in the `languageServerHome` setting holds no JDT LS
/// installation for this platform.
pub const LAUNCH_SERVER_INVALID: &str = "launch.server_invalid";

/// Plugin setting naming the JDT LS folder; empty for automatic discovery.
pub const SERVER_HOME_SETTING: &str = "languageServerHome";
/// Environment variable that may name the JDT LS folder.
pub const SERVER_HOME_VARIABLE: &str = "JDTLS_HOME";
/// Directory of an installation that holds the Equinox launcher JAR.
pub const PLUGINS_DIRECTORY: &str = "plugins";
/// File name prefix of the Equinox launcher JAR; the version follows it.
pub const LAUNCHER_PREFIX: &str = "org.eclipse.equinox.launcher_";
/// File name suffix of the Equinox launcher JAR.
pub const LAUNCHER_SUFFIX: &str = ".jar";
/// Subdirectory Homebrew installs the JDT LS files into.
const LIBEXEC_DIRECTORY: &str = "libexec";
/// The only file Equinox reads from a platform configuration directory.
const CONFIG_INI: &str = "config.ini";
/// Folder name prefix of the VS Code Java extension; its version follows.
const EDITOR_EXTENSION_PREFIX: &str = "redhat.java-";
/// Directory of the VS Code Java extension that holds JDT LS.
const EDITOR_EXTENSION_SERVER: &str = "server";
/// Extension directories of VS Code and the editors built from it, below the
/// user's home directory.
const EDITOR_EXTENSION_ROOTS: &[&str] = &[
    ".vscode/extensions",
    ".vscode-insiders/extensions",
    ".vscode-oss/extensions",
    ".cursor/extensions",
];
/// Most entries read from one editor extension directory, so a large one
/// delays the launch by a bounded amount.
const EDITOR_EXTENSION_ENTRY_BUDGET: usize = 4096;

/// Message of [`LAUNCH_SERVER_MISSING`].
///
/// TODO(host-api: API-37 i18n): user-facing text must go through the host's
/// i18n table.
pub const SERVER_MISSING_MESSAGE: &str = "Java Support needs the Eclipse JDT Language Server, \
which was not found on this computer. Install it (for example with `brew install jdtls`), \
or choose its folder in Settings > Languages & Frameworks > Java > Language server folder.";
/// Message of [`LAUNCH_SERVER_INVALID`].
///
/// TODO(host-api: API-37 i18n): user-facing text must go through the host's
/// i18n table.
pub const SERVER_INVALID_MESSAGE: &str = "The language server folder chosen in the Java \
settings does not contain the Eclipse JDT Language Server for this platform. Choose the \
folder that contains plugins and the config folders, or clear the setting to find JDT LS \
automatically.";

/// The JDT LS files a launch uses, absolute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInstallation {
    /// The installation folder: the parent of `plugins/`.
    pub home: PathBuf,
    /// The Equinox launcher JAR passed after `-jar`.
    pub launcher_jar: PathBuf,
    /// The read-only Equinox configuration of the platform; the launch uses a
    /// writable copy of it.
    pub configuration: PathBuf,
}

/// What [`locate`] reads from the backend's process environment, so tests
/// can supply their own.
#[derive(Debug, Clone, Default)]
pub struct SearchEnvironment {
    /// The user's home directory.
    pub home: Option<PathBuf>,
    /// `JDTLS_HOME`.
    pub server_home: Option<PathBuf>,
    /// `PATH`.
    pub path: Option<OsString>,
    /// `LOCALAPPDATA`, where mason.nvim installs on Windows.
    pub local_app_data: Option<PathBuf>,
}

impl SearchEnvironment {
    /// The environment of the running backend.
    pub fn current() -> Self {
        let directory = |name: &str| {
            std::env::var_os(name)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
        };
        SearchEnvironment {
            home: directory(if cfg!(windows) { "USERPROFILE" } else { "HOME" }),
            server_home: directory(SERVER_HOME_VARIABLE),
            path: std::env::var_os("PATH"),
            local_app_data: directory("LOCALAPPDATA"),
        }
    }
}

/// The `config_<platform>` directory JDT LS ships for `os` (`macos`,
/// `linux` or `windows`, as `resolveParams.platform.os` names it) and `arch`
/// (`arm64` or `x86_64`). JDT LS ships one Windows configuration for every
/// architecture.
pub fn configuration_name(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("macos", "arm64") => Some("config_mac_arm"),
        ("macos", "x86_64") => Some("config_mac"),
        ("linux", "arm64") => Some("config_linux_arm"),
        ("linux", "x86_64") => Some("config_linux"),
        ("windows", _) => Some("config_win"),
        _ => None,
    }
}

/// The installation in `folder` or in its `libexec/`, or `None` when neither
/// holds one for the configuration `configuration`.
pub fn inspect(folder: &Path, configuration: &str) -> Option<ServerInstallation> {
    [folder.to_path_buf(), folder.join(LIBEXEC_DIRECTORY)]
        .into_iter()
        .find_map(|home| installation_at(&home, configuration))
}

fn installation_at(home: &Path, configuration: &str) -> Option<ServerInstallation> {
    // Java cannot load `-jar` or `-javaagent` through a Windows verbatim
    // prefix, so the paths keep their ordinary spelling.
    let home = dunce::canonicalize(home).ok()?;
    let launcher_jar = launcher_in(&home.join(PLUGINS_DIRECTORY))?;
    let configuration = home.join(configuration);
    if !configuration.join(CONFIG_INI).is_file() {
        return None;
    }
    Some(ServerInstallation {
        home,
        launcher_jar,
        configuration,
    })
}

/// The one Equinox launcher JAR in `directory`. None or several mean the
/// folder is no usable installation.
fn launcher_in(directory: &Path) -> Option<PathBuf> {
    let mut launchers: Vec<PathBuf> = fs::read_dir(directory)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with(LAUNCHER_PREFIX) && name.ends_with(LAUNCHER_SUFFIX)
                })
                && path.is_file()
        })
        .collect();
    if launchers.len() == 1 {
        launchers.pop()
    } else {
        None
    }
}

/// Finds the JDT LS installation for `os` and `arch`.
///
/// `configured` is the `languageServerHome` setting, `None` when it is empty.
pub fn locate(
    configured: Option<&Path>,
    environment: &SearchEnvironment,
    os: &str,
    arch: &str,
) -> Result<ServerInstallation, PluginError> {
    let configuration = configuration_name(os, arch)
        .ok_or_else(|| PluginError::new(LAUNCH_SERVER_MISSING, SERVER_MISSING_MESSAGE))?;
    if let Some(folder) = configured {
        return inspect(folder, configuration)
            .ok_or_else(|| PluginError::new(LAUNCH_SERVER_INVALID, SERVER_INVALID_MESSAGE));
    }
    candidates(environment, os)
        .iter()
        .find_map(|folder| inspect(folder, configuration))
        .ok_or_else(|| PluginError::new(LAUNCH_SERVER_MISSING, SERVER_MISSING_MESSAGE))
}

/// The folders [`locate`] tries without a configured one, in order. A folder
/// may hold no installation; [`inspect`] decides.
pub fn candidates(environment: &SearchEnvironment, os: &str) -> Vec<PathBuf> {
    let mut folders = Vec::new();
    folders.extend(environment.server_home.clone());
    folders.extend(launcher_installations(environment.path.as_ref(), os));
    folders.extend(package_manager_folders(environment, os));
    if let Some(home) = &environment.home {
        folders.extend(editor_extension_servers(home));
    }
    folders
}

/// The installation folders of the `jdtls` launchers on `path`: a launcher is
/// `<installation>/bin/jdtls`, possibly behind symbolic links.
fn launcher_installations(path: Option<&OsString>, os: &str) -> Vec<PathBuf> {
    let Some(path) = path else {
        return Vec::new();
    };
    let names: &[&str] = if os == "windows" {
        &["jdtls.bat", "jdtls.cmd"]
    } else {
        &["jdtls"]
    };
    std::env::split_paths(path)
        .filter(|directory| directory.is_absolute())
        .flat_map(|directory| names.iter().map(move |name| directory.join(name)))
        .filter(|launcher| launcher.is_file())
        .filter_map(|launcher| dunce::canonicalize(launcher).ok())
        .filter_map(|launcher| launcher.parent()?.parent().map(Path::to_path_buf))
        .collect()
}

/// The folders package managers install JDT LS into on `os`.
fn package_manager_folders(environment: &SearchEnvironment, os: &str) -> Vec<PathBuf> {
    let mut folders = Vec::new();
    let home = environment.home.as_deref();
    match os {
        "macos" => {
            folders.push(PathBuf::from("/opt/homebrew/opt/jdtls"));
            folders.push(PathBuf::from("/usr/local/opt/jdtls"));
        }
        "linux" => {
            folders.push(PathBuf::from("/home/linuxbrew/.linuxbrew/opt/jdtls"));
            folders.extend(home.map(|home| home.join(".linuxbrew/opt/jdtls")));
        }
        _ => {}
    }
    if os == "windows" {
        folders.extend(
            environment
                .local_app_data
                .as_deref()
                .map(|data| data.join("nvim-data/mason/packages/jdtls")),
        );
        folders.extend(home.map(|home| home.join("scoop/apps/jdtls/current")));
    } else {
        folders.extend(home.map(|home| home.join(".local/share/nvim/mason/packages/jdtls")));
    }
    folders
}

/// The `server/` folders of the VS Code Java extensions under `home`, newest
/// extension version first.
fn editor_extension_servers(home: &Path) -> Vec<PathBuf> {
    let mut extensions: Vec<(Vec<u64>, PathBuf)> = Vec::new();
    for root in EDITOR_EXTENSION_ROOTS {
        let Ok(entries) = fs::read_dir(home.join(root)) else {
            continue;
        };
        for entry in entries
            .filter_map(Result::ok)
            .take(EDITOR_EXTENSION_ENTRY_BUDGET)
        {
            let name = entry.file_name();
            let Some(version) = name
                .to_str()
                .and_then(|name| name.strip_prefix(EDITOR_EXTENSION_PREFIX))
                .and_then(extension_version)
            else {
                continue;
            };
            extensions.push((version, entry.path().join(EDITOR_EXTENSION_SERVER)));
        }
    }
    // Newest first; the path keeps the order stable for equal versions.
    extensions.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    extensions.into_iter().map(|(_, folder)| folder).collect()
}

/// The dotted version that starts an extension folder name after its
/// publisher and name, such as `1.58.0` in `1.58.0-darwin-arm64`.
fn extension_version(text: &str) -> Option<Vec<u64>> {
    let version = text.split('-').next()?;
    let parts: Option<Vec<u64>> = version.split('.').map(|part| part.parse().ok()).collect();
    parts.filter(|parts| !parts.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    const LAUNCHER: &str = "org.eclipse.equinox.launcher_1.7.100.v20251111-0406.jar";

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "lithe-java-jdtls-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            TempDir(dunce::canonicalize(path).unwrap())
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Lays out a JDT LS installation with the configurations of `configurations`.
    fn install(home: &Path, configurations: &[&str]) {
        fs::create_dir_all(home.join(PLUGINS_DIRECTORY)).unwrap();
        fs::write(home.join(PLUGINS_DIRECTORY).join(LAUNCHER), b"").unwrap();
        for configuration in configurations {
            fs::create_dir_all(home.join(configuration)).unwrap();
            fs::write(
                home.join(configuration).join(CONFIG_INI),
                b"osgi.bundles=x\n",
            )
            .unwrap();
        }
    }

    fn environment(home: &Path) -> SearchEnvironment {
        SearchEnvironment {
            home: Some(home.to_path_buf()),
            ..SearchEnvironment::default()
        }
    }

    #[test]
    fn every_supported_platform_has_its_configuration() {
        assert_eq!(configuration_name("macos", "arm64"), Some("config_mac_arm"));
        assert_eq!(configuration_name("macos", "x86_64"), Some("config_mac"));
        assert_eq!(
            configuration_name("linux", "arm64"),
            Some("config_linux_arm")
        );
        assert_eq!(configuration_name("linux", "x86_64"), Some("config_linux"));
        assert_eq!(configuration_name("windows", "x86_64"), Some("config_win"));
        assert_eq!(configuration_name("windows", "arm64"), Some("config_win"));
        assert_eq!(configuration_name("freebsd", "x86_64"), None);
    }

    #[test]
    fn an_installation_needs_one_launcher_and_the_platform_configuration() {
        let temp = TempDir::new();
        let home = temp.0.join("jdtls");
        install(&home, &["config_mac_arm"]);
        let found = inspect(&home, "config_mac_arm").unwrap();
        assert_eq!(found.home, home);
        assert_eq!(found.launcher_jar, home.join("plugins").join(LAUNCHER));
        assert_eq!(found.configuration, home.join("config_mac_arm"));

        // Another platform's configuration does not count.
        assert!(inspect(&home, "config_linux").is_none());
        // Two launchers leave the choice open.
        fs::write(home.join("plugins/org.eclipse.equinox.launcher_9.jar"), b"").unwrap();
        assert!(inspect(&home, "config_mac_arm").is_none());
        assert!(inspect(&temp.0.join("missing"), "config_mac_arm").is_none());
    }

    #[test]
    fn a_homebrew_keg_is_found_through_libexec() {
        let temp = TempDir::new();
        let keg = temp.0.join("Cellar/jdtls/1.60.0");
        install(&keg.join("libexec"), &["config_mac_arm"]);
        assert_eq!(
            inspect(&keg, "config_mac_arm").unwrap().home,
            keg.join("libexec")
        );
    }

    #[test]
    fn a_configured_folder_is_final() {
        let temp = TempDir::new();
        let chosen = temp.0.join("chosen");
        install(&chosen, &["config_linux"]);
        let discovered = temp.0.join("discovered");
        install(&discovered, &["config_linux"]);
        let mut search = environment(&temp.0);
        search.server_home = Some(discovered.clone());

        let found = locate(Some(&chosen), &search, "linux", "x86_64").unwrap();
        assert_eq!(found.home, chosen);
        // An unusable choice fails instead of falling back to another installation.
        let error = locate(Some(&temp.0.join("empty")), &search, "linux", "x86_64").unwrap_err();
        assert_eq!(error.code(), LAUNCH_SERVER_INVALID);
        assert_eq!(error.message(), SERVER_INVALID_MESSAGE);
        // Without a choice the discovered installation is used.
        assert_eq!(
            locate(None, &search, "linux", "x86_64").unwrap().home,
            discovered
        );
    }

    #[test]
    fn nothing_found_is_one_error_that_names_no_path() {
        let temp = TempDir::new();
        let error = locate(None, &environment(&temp.0), "windows", "x86_64").unwrap_err();
        assert_eq!(error.code(), LAUNCH_SERVER_MISSING);
        assert_eq!(error.message(), SERVER_MISSING_MESSAGE);
        assert!(!error.message().contains(temp.0.to_string_lossy().as_ref()));
        let error = locate(None, &environment(&temp.0), "plan9", "x86_64").unwrap_err();
        assert_eq!(error.code(), LAUNCH_SERVER_MISSING);
    }

    #[cfg(unix)]
    #[test]
    fn a_launcher_on_path_leads_to_its_installation() {
        let temp = TempDir::new();
        let keg = temp.0.join("Cellar/jdtls/1.63.0");
        install(&keg.join("libexec"), &["config_linux"]);
        fs::create_dir_all(keg.join("bin")).unwrap();
        fs::write(keg.join("bin/jdtls"), b"#!/bin/sh\n").unwrap();
        let bin = temp.0.join("bin");
        fs::create_dir_all(&bin).unwrap();
        std::os::unix::fs::symlink(keg.join("bin/jdtls"), bin.join("jdtls")).unwrap();
        let mut search = environment(&temp.0);
        search.path = Some(std::env::join_paths([temp.0.join("nothing"), bin]).unwrap());
        assert_eq!(
            locate(None, &search, "linux", "x86_64").unwrap().home,
            keg.join("libexec")
        );
    }

    #[test]
    fn mason_is_searched_in_the_home_directory() {
        let temp = TempDir::new();
        let mason = temp.0.join(".local/share/nvim/mason/packages/jdtls");
        install(&mason, &["config_linux_arm"]);
        assert_eq!(
            locate(None, &environment(&temp.0), "linux", "arm64")
                .unwrap()
                .home,
            mason
        );
    }

    #[test]
    fn the_newest_editor_extension_wins() {
        let temp = TempDir::new();
        let extensions = temp.0.join(".vscode/extensions");
        for name in [
            "redhat.java-1.9.0-darwin-arm64",
            "redhat.java-1.58.0-darwin-arm64",
            "redhat.java-1.47.0",
        ] {
            install(&extensions.join(name).join("server"), &["config_win"]);
        }
        fs::create_dir_all(extensions.join("redhat.java-latest")).unwrap();
        fs::create_dir_all(extensions.join("vscjava.vscode-java-debug-0.60.0")).unwrap();
        assert_eq!(
            editor_extension_servers(&temp.0),
            vec![
                extensions.join("redhat.java-1.58.0-darwin-arm64/server"),
                extensions.join("redhat.java-1.47.0/server"),
                extensions.join("redhat.java-1.9.0-darwin-arm64/server"),
            ]
        );
        assert_eq!(
            // Windows names no machine-wide folder, so only the extensions count.
            locate(None, &environment(&temp.0), "windows", "x86_64")
                .unwrap()
                .home,
            extensions.join("redhat.java-1.58.0-darwin-arm64/server")
        );
    }

    #[test]
    fn jdtls_home_comes_before_the_other_folders() {
        let temp = TempDir::new();
        let mut search = environment(&temp.0);
        search.server_home = Some(temp.0.join("explicit"));
        let folders = candidates(&search, "macos");
        assert_eq!(folders[0], temp.0.join("explicit"));
        assert!(folders.contains(&PathBuf::from("/opt/homebrew/opt/jdtls")));
        let folders = candidates(&search, "windows");
        assert!(folders.contains(&temp.0.join("scoop/apps/jdtls/current")));
        assert!(!folders.contains(&PathBuf::from("/opt/homebrew/opt/jdtls")));
    }
}

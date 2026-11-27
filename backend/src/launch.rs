//! Backend-resolved launch of JDT LS (`launch.kind: "backend"`).
//!
//! Before the host starts the `java` language server it calls
//! [`LAUNCH_FUNCTION_ID`] with the contract's `resolveParams` and the
//! workspace `context`, and it starts the process the answer describes. This
//! module does what the host's `jdt` integration did: it finds the user's
//! JDT LS installation, gives Equinox a writable copy of its configuration,
//! and builds the JDT LS command line, its initialization options and its
//! settings.
//!
//! # Where the files are
//!
//! JDT LS and its JDK live outside the small plugin package. The host prepares
//! the dependencies declared in `managedRuntimes`; [`crate::managed_runtimes`]
//! qualifies existing files before any missing dependency is downloaded. The
//! launch uses that approved server home and the separate `jdtls-runtime`
//! tool ([`SERVER_RUNTIME_TOOL`]). The project's `java` tool stays independent.
//!
//! The package only carries what the plugin adds to JDT LS: the Lombok agent
//! and the Java Debug and Java Test extension bundles. `resolveParams` carries
//! no package directory. The manifest installs the backend at
//! `bin/lithe-java-backend`, so the package root is the parent of the
//! directory of the running executable ([`package_root`]), and every package
//! path below is relative to it.
//!
//! # Where the launch writes
//!
//! The package and the JDT LS installation are read-only: neither the backend
//! nor JDT LS writes into them. The launch writes only into the four
//! directories the host provides for this start, and the answer lists them as
//! `writablePaths`:
//!
//! - `stateDirectory`: the Equinox configuration area, one per `config.ini`
//!   digest, which the backend prepares before it answers;
//! - `workspaceStateDirectory`: the `-data` directory of the workspace;
//! - `logDirectory`: the Equinox log `jdt.log` and its backups;
//! - `temporaryDirectory`: `java.io.tmpdir` of the JDT LS process.
//!
//! # Settings
//!
//! The answer's `workspaceConfiguration` is
//! [`settings_session::java_configuration`] of the settings the session knows
//! for the workspace, the same value a later `workspace/didChangeConfiguration`
//! push sends, so a start and a push never disagree. The initialization
//! options carry the same `settings.java`, because JDT LS imports the
//! projects while it handles `initialize`.
//!
//! [`settings_session::java_configuration`]: crate::settings_session::java_configuration

use crate::jdtls::{
    ServerInstallation, LAUNCH_SERVER_INVALID, LAUNCH_SERVER_MISSING, SERVER_HOME_SETTING,
};
use crate::managed_runtimes;
use crate::project_model::{ModelOptions, ProjectModel};
use crate::run::file_uri;
use crate::settings_session::{java_configuration, jdt_settings, SettingsSession, SERVER_ID};
use lithe_java_domain::jdt_integration::launch_resources::{bundles_in, missing_resource_message};
use lithe_java_domain::lsp::languages::jdt::{
    adapt_initialization_options, add_trigger_files, direct_java_arguments, jdt_java_runtimes,
    JdtDirectLaunchResources, JdtJavaRuntime,
};
use lithe_java_domain::lsp::languages::prepare_jdt_configuration_area;
use lithe_java_domain::lsp::JavaRuntimeCandidate;
use lithe_plugin_sdk::{
    Context, FunctionError, Host, PluginError, SdkError, Value, WorkspaceContext,
};
use serde_json::json;
use std::collections::VecDeque;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Plugin function the manifest names as the server's `launch.function`.
pub const LAUNCH_FUNCTION_ID: &str = "java.resolveLaunch";

/// `java.resolveLaunch` was called without a usable `resolveParams` field:
/// another server ID, an unknown architecture, a directory that is not
/// absolute, or a setting outside its declaration.
pub const LAUNCH_INVALID_PARAMS: &str = "launch.invalid_params";
/// The backend cannot find the package it was installed from.
pub const LAUNCH_PACKAGE_UNAVAILABLE: &str = "launch.package_unavailable";
/// The package lacks a file the launch needs; reinstalling repairs it.
pub const LAUNCH_RESOURCE_MISSING: &str = "launch.resource_missing";
/// The writable Equinox configuration area could not be prepared.
pub const LAUNCH_CONFIGURATION_FAILED: &str = "launch.configuration_failed";
/// The JDK the host resolved for [`SERVER_RUNTIME_TOOL`] is older than
/// [`MINIMUM_SERVER_JAVA_VERSION`].
pub const LAUNCH_RUNTIME_TOO_OLD: &str = "launch.runtime_too_old";

/// Every `pluginErrorCode` `java.resolveLaunch` answers with, one per cause.
pub const LAUNCH_ERROR_CODES: &[&str] = &[
    LAUNCH_INVALID_PARAMS,
    LAUNCH_PACKAGE_UNAVAILABLE,
    LAUNCH_RESOURCE_MISSING,
    LAUNCH_CONFIGURATION_FAILED,
    LAUNCH_SERVER_MISSING,
    LAUNCH_SERVER_INVALID,
    LAUNCH_RUNTIME_TOO_OLD,
];

/// The plugin's tool definition of the JDK that runs JDT LS. It is not the
/// `java` tool: that one is the project's JDK, which may be far older than
/// JDT LS accepts.
pub const SERVER_RUNTIME_TOOL: &str = "jdtls-runtime";
/// Oldest Java feature release JDT LS runs on.
pub const MINIMUM_SERVER_JAVA_VERSION: u64 = managed_runtimes::JAVA_FEATURE;
/// Message of [`LAUNCH_RUNTIME_TOO_OLD`] for [`MINIMUM_SERVER_JAVA_VERSION`].
///
/// TODO(host-api: API-37 i18n): user-facing text must go through the host's
/// i18n table.
pub const RUNTIME_TOO_OLD_MESSAGE: &str = "The Eclipse JDT Language Server needs Java 25 or \
later, but the JDK chosen for it is older. Choose a JDK 25 or later for \"JDK for JDT LS\" in \
Settings > Build, Execution, Deployment > Toolchains.";

/// Message of [`LAUNCH_RUNTIME_TOO_OLD`] for the Java feature release
/// `minimum_java_version` the selected JDT LS installation requires.
pub fn runtime_too_old_message(minimum_java_version: u64) -> String {
    let minimum = minimum_java_version.max(MINIMUM_SERVER_JAVA_VERSION);
    format!(
        "The Eclipse JDT Language Server needs Java {minimum} or later, but the JDK chosen for \
it is older. Choose a JDK {minimum} or later for \"JDK for JDT LS\" in Settings > Build, \
Execution, Deployment > Toolchains."
    )
}

/// Configuration areas of older JDT LS versions could not be removed. Only
/// disk space is at stake, so the launch goes on.
pub const LAUNCH_CONFIGURATION_CLEANUP_FAILED: &str = "launch.configuration_cleanup_failed";

/// Diagnostics categories a launch reports as `degraded` without failing.
/// The host did not answer `workspace/getFolders` for the workspace, so the
/// launch names no `triggerFiles` and JDT LS falls back to the first opened
/// file to lay out a workspace without a build file.
pub const LAUNCH_WORKSPACE_FOLDERS_UNAVAILABLE: &str = "launch.workspace_folders_unavailable";
/// The project needs servlet container libraries (a web module, a Server
/// Runtime container) and none was found, so `javax.servlet` stays
/// unresolved until the user configures one.
pub const LAUNCH_SERVLET_CONTAINER_MISSING: &str = "launch.servlet_container_missing";
/// Libraries the project model declares could not be added; the count is
/// how many. The Java libraries settings page lists them.
pub const LAUNCH_PROJECT_MODEL_UNRESOLVED: &str = "launch.project_model_unresolved";
/// The project model scan stopped at its budget; deeper layouts were not read.
pub const LAUNCH_PROJECT_MODEL_TRUNCATED: &str = "launch.project_model_truncated";
/// Directory scanning lost some optional trigger-file hints to I/O errors.
/// Missing files and directories during concurrent editing are not failures.
pub const LAUNCH_TRIGGER_SCAN_UNREADABLE: &str = "launch.trigger_scan_unreadable";
/// Every degraded-diagnostic category `java.resolveLaunch` reports.
pub const LAUNCH_CATEGORIES: &[&str] = &[
    LAUNCH_CONFIGURATION_CLEANUP_FAILED,
    LAUNCH_WORKSPACE_FOLDERS_UNAVAILABLE,
    LAUNCH_SERVLET_CONTAINER_MISSING,
    LAUNCH_PROJECT_MODEL_UNRESOLVED,
    LAUNCH_PROJECT_MODEL_TRUNCATED,
    LAUNCH_TRIGGER_SCAN_UNREADABLE,
];

/// Most `.java` files named as `triggerFiles`. JDT LS uses the first one
/// under each workspace folder, so one per folder would do; a few more let it
/// still find one when the first lies in a directory it ignores.
pub const TRIGGER_FILE_LIMIT: usize = 16;
/// How deep below a workspace folder the trigger-file scan descends. Source
/// roots of plain workspaces sit at the top or one level down (`src`,
/// `src/main/java`), and the package directories below them are what the
/// depth must still cover.
pub const TRIGGER_SCAN_DEPTH: usize = 8;
/// Most directory entries the scan reads in total, so a huge workspace delays
/// the launch by a bounded amount instead of by its size.
pub const TRIGGER_SCAN_ENTRY_BUDGET: usize = 20_000;
/// Directories the scan never enters: hidden ones, dependency and build
/// output trees. A `.java` file there is generated or foreign and would make
/// JDT LS lay the project out around it.
pub const TRIGGER_SCAN_SKIPPED_DIRECTORIES: &[&str] = &[
    "node_modules",
    "target",
    "build",
    "bin",
    "out",
    "dist",
    "vendor",
];

/// `heapMaxMB` when the settings do not carry it; the manifest's default.
pub const DEFAULT_HEAP_MAX_MB: u64 = 2048;
/// Smallest `heapMaxMB` the manifest accepts. JDT LS imports in a
/// constrained-memory mode at or below 1536 MB.
pub const MIN_HEAP_MAX_MB: u64 = 2048;
/// Largest `heapMaxMB` the manifest accepts.
pub const MAX_HEAP_MAX_MB: u64 = 16384;
/// `logLevel` when the settings do not carry it; the manifest's default.
pub const DEFAULT_LOG_LEVEL: &str = "WARNING";
/// The `logLevel` values the manifest declares.
pub const LOG_LEVELS: &[&str] = &["ALL", "INFO", "WARNING", "SEVERE"];
/// The `platform.arch` values the contract defines.
pub const ARCHITECTURES: &[&str] = &["arm64", "x86_64"];
/// Gate that holds the backend's requests until JDT LS reports its Java
/// service ready.
pub const SERVICE_READY_GATE_ID: &str = "service-ready";

// Package layout. `plugin.json` does not name these files (a backend
// launch declares only the function); this module is the single source for
// where `scripts/assemble-package.sh` puts them and `scripts/verify-package.sh`
// checks them, and `backend/tests/repository_pins.rs` holds both scripts to
// [`PACKAGE_LAYOUT`].

/// Package directory of everything the plugin adds to the user's JDT LS.
pub const EXTENSIONS_DIRECTORY: &str = "jdtls-extensions";
/// Package path of the Lombok agent JDT LS runs with.
pub const LOMBOK_AGENT: &str = "jdtls-extensions/lombok/lombok.jar";
/// Package directories whose JAR files JDT LS loads as extension bundles, in
/// this order and then by file name: the Java Debug Server, the Java Test
/// extensions, then the plugin's own project model bundle, built from
/// `jdtls-extension/project-model` by `scripts/package/project-model-bundle.mjs`.
pub const EXTENSION_BUNDLE_DIRECTORIES: &[&str] = &[
    "jdtls-extensions/java-debug",
    "jdtls-extensions/java-test/extensions",
    "jdtls-extensions/project-model",
];
/// Package path of the Java Test runner JAR the test flow adds to the class
/// path of TestNG launches; JDT LS itself does not load it.
pub const TEST_RUNNER: &str =
    "jdtls-extensions/java-test/runner/com.microsoft.java.test.runner-jar-with-dependencies.jar";
/// Every package path the backend resolves at launch or test time, so the
/// package scripts can be held to the same list.
pub const PACKAGE_LAYOUT: &[&str] = &[
    LOMBOK_AGENT,
    EXTENSION_BUNDLE_DIRECTORIES[0],
    EXTENSION_BUNDLE_DIRECTORIES[1],
    EXTENSION_BUNDLE_DIRECTORIES[2],
    TEST_RUNNER,
];
/// Directory under `workspaceStateDirectory` passed to JDT LS as `-data`.
const DATA_DIRECTORY: &str = "data";
/// Equinox log file under `logDirectory`.
const LOG_FILE: &str = "jdt.log";
/// JDT LS notification that reports the readiness of its Java service.
const LANGUAGE_STATUS: &str = "language/status";
/// `language/status` type JDT LS sends once the projects are imported.
const SERVICE_READY: &str = "ServiceReady";

/// The `resolveParams` of one start, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRequest {
    /// `platform.os`: `macos`, `linux` or `windows`.
    pub os: String,
    /// `platform.arch`: `arm64` or `x86_64`. With the operating system it
    /// picks the Equinox configuration of the installation.
    pub architecture: String,
    /// `stateDirectory`: the plugin's writable directory for this server.
    pub state_directory: PathBuf,
    /// `workspaceStateDirectory`: the writable directory of this workspace.
    pub workspace_state_directory: PathBuf,
    /// `logDirectory`: the session's log directory for the plugin.
    pub log_directory: PathBuf,
    /// `temporaryDirectory`: scratch space the host removes after the process.
    pub temporary_directory: PathBuf,
    /// `settings.heapMaxMB`: the JVM's maximum heap, in MiB.
    pub heap_max_mb: u64,
    /// `settings.logLevel`: the JDT LS log level.
    pub log_level: String,
    /// Explicit `settings.languageServerHome`, otherwise the approved
    /// `runtimes.jdtls.home`. No launch-time rediscovery can replace it.
    pub server_home: Option<PathBuf>,
    /// `tools["jdtls-runtime"].version`, when the host resolved the JDK that
    /// runs JDT LS and read its version.
    pub server_runtime_version: Option<String>,
    /// `tools.java`, the installation the host resolved for the plugin's
    /// `java` tool, as the server's default runtime: Run starts the project
    /// with that JDK, so the server must compile for it. Empty when the host
    /// found none, which leaves the server on the runtimes it already knows.
    pub java_runtimes: Vec<JdtJavaRuntime>,
    /// `settings.projectModelDetection` and `settings.servletContainerHome`.
    pub model_options: ModelOptions,
}

/// The files of a JDT LS launch, absolute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchFiles {
    /// The Equinox launcher JAR of the user's installation, passed after
    /// `-jar`.
    pub launcher_jar: PathBuf,
    /// The packaged Lombok agent passed as `-javaagent`.
    pub lombok_agent: PathBuf,
    /// The read-only Equinox configuration of the installation for this
    /// platform; the launch uses a writable copy of it.
    pub server_configuration: PathBuf,
    /// The packaged extension bundle JARs, in load order.
    pub extension_bundles: Vec<PathBuf>,
}

/// Checks the `resolveParams` of a call.
pub fn parse_launch_params(params: &Value) -> Result<LaunchRequest, PluginError> {
    if params.get("serverID").and_then(Value::as_str) != Some(SERVER_ID) {
        return Err(invalid_params(
            "The launch is not for the Java language server.",
        ));
    }
    let architecture = params
        .get("platform")
        .and_then(|platform| platform.get("arch"))
        .and_then(Value::as_str)
        .filter(|arch| ARCHITECTURES.contains(arch))
        .ok_or_else(|| invalid_params("The launch names no supported architecture."))?;
    let os = params
        .get("platform")
        .and_then(|platform| platform.get("os"))
        .and_then(Value::as_str)
        .filter(|os| !os.is_empty())
        .ok_or_else(|| invalid_params("The launch names no operating system."))?;
    let settings = params.get("settings").cloned().unwrap_or_else(|| json!({}));
    let heap_max_mb = match settings.get("heapMaxMB") {
        None | Some(Value::Null) => DEFAULT_HEAP_MAX_MB,
        Some(value) => value
            .as_u64()
            .filter(|mb| (MIN_HEAP_MAX_MB..=MAX_HEAP_MAX_MB).contains(mb))
            .ok_or_else(|| invalid_params("The maximum heap setting is out of range."))?,
    };
    let log_level = match settings.get("logLevel") {
        None | Some(Value::Null) => DEFAULT_LOG_LEVEL,
        Some(value) => value
            .as_str()
            .filter(|level| LOG_LEVELS.contains(level))
            .ok_or_else(|| invalid_params("The log level setting is not supported."))?,
    };
    let server_home = match settings.get(SERVER_HOME_SETTING) {
        None | Some(Value::Null) => None,
        Some(value) => match value.as_str() {
            Some("") => None,
            Some(folder) if Path::new(folder).is_absolute() => Some(PathBuf::from(folder)),
            _ => {
                return Err(invalid_params(
                    "The language server folder setting is not a folder.",
                ))
            }
        },
    };
    let prepared_home = match params.get("runtimes").and_then(|value| value.get("jdtls")) {
        None => None,
        Some(value) => Some(
            value
                .get("home")
                .and_then(Value::as_str)
                .map(PathBuf::from)
                .filter(|home| home.is_absolute())
                .ok_or_else(|| invalid_params("The prepared language server folder is invalid."))?,
        ),
    };
    let model_options = crate::project_model::parse_options(&settings).map_err(invalid_params)?;
    Ok(LaunchRequest {
        os: os.to_string(),
        architecture: architecture.to_string(),
        state_directory: directory(params, "stateDirectory")?,
        workspace_state_directory: directory(params, "workspaceStateDirectory")?,
        log_directory: directory(params, "logDirectory")?,
        temporary_directory: directory(params, "temporaryDirectory")?,
        heap_max_mb,
        log_level: log_level.to_string(),
        server_home: server_home.or(prepared_home),
        server_runtime_version: host_tool_version(params, SERVER_RUNTIME_TOOL),
        java_runtimes: jdt_java_runtimes(&host_java_tool(params)),
        model_options,
    })
}

/// The `version` of the `tool` installation in the optional `tools` of the
/// request. The contract promises no field, so a missing one is no error.
fn host_tool_version(params: &Value, tool: &str) -> Option<String> {
    params
        .get("tools")?
        .get(tool)?
        .get("version")?
        .as_str()
        .map(str::trim)
        .filter(|version| !version.is_empty())
        .map(str::to_string)
}

/// The Java feature release of `request.server_runtime_version`, when readable.
pub fn server_runtime_feature(request: &LaunchRequest) -> Option<u64> {
    request
        .server_runtime_version
        .as_deref()
        .and_then(|version| version.split(['.', '_', '+', '-']).next())
        .and_then(|feature| feature.parse::<u64>().ok())
}

/// Refuses a JDT LS runtime the host reported as older than
/// [`MINIMUM_SERVER_JAVA_VERSION`]. A runtime without a readable version is
/// left to the host and to JDT LS, which reports what it cannot run on.
pub fn check_server_runtime(request: &LaunchRequest) -> Result<(), PluginError> {
    check_server_runtime_for(request, MINIMUM_SERVER_JAVA_VERSION)
}

/// Refuses a JDT LS runtime the host reported as older than
/// `minimum_java_version` (floored at [`MINIMUM_SERVER_JAVA_VERSION`]).
pub fn check_server_runtime_for(
    request: &LaunchRequest,
    minimum_java_version: u64,
) -> Result<(), PluginError> {
    let minimum = minimum_java_version.max(MINIMUM_SERVER_JAVA_VERSION);
    match server_runtime_feature(request) {
        Some(feature) if feature < minimum => Err(PluginError::new(
            LAUNCH_RUNTIME_TOO_OLD,
            runtime_too_old_message(minimum),
        )),
        _ => Ok(()),
    }
}

/// The `java` installation the host resolved, from the optional `tools`
/// of the request. The contract has the host leave out a tool it could not
/// find and promises only the fields it knows, so a missing or incomplete
/// entry is no error: without a home or a version, the server cannot be
/// pointed at the JDK, and the list stays empty.
fn host_java_tool(params: &Value) -> Vec<JavaRuntimeCandidate> {
    let Some(tool) = params.get("tools").and_then(|tools| tools.get("java")) else {
        return Vec::new();
    };
    let field = |name: &str| {
        tool.get(name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    };
    match (field("home"), field("version")) {
        (Some(home), Some(version)) => vec![JavaRuntimeCandidate {
            is_default: true,
            home_path: home.to_string(),
            version: version.to_string(),
        }],
        _ => Vec::new(),
    }
}

/// An absolute directory field of `resolveParams`, kept as the host wrote
/// it: the host compares `writablePaths` with it as text.
fn directory(params: &Value, field: &str) -> Result<PathBuf, PluginError> {
    params
        .get(field)
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| invalid_params("The launch is missing a writable directory."))
}

fn invalid_params(message: &str) -> PluginError {
    PluginError::new(LAUNCH_INVALID_PARAMS, message)
}

fn missing_resource(name: &str) -> PluginError {
    PluginError::new(LAUNCH_RESOURCE_MISSING, missing_resource_message(name))
}

/// Root of the installed package: the parent of `bin/`, where the manifest
/// installs the backend executable.
pub fn package_root() -> Result<PathBuf, PluginError> {
    let unavailable = || {
        PluginError::new(
            LAUNCH_PACKAGE_UNAVAILABLE,
            "The Java Support plugin cannot find its installed files. Reinstall the plugin.",
        )
    };
    let executable = std::env::current_exe()
        .and_then(fs::canonicalize)
        .map_err(|_| unavailable())?;
    package_root_from_executable(&executable).ok_or_else(unavailable)
}

// Java instrumentation cannot load -javaagent through a Windows verbatim
// prefix. Simplify only when an ordinary spelling has the same identity;
// canonicalization above still owns symlink resolution.
fn package_root_from_executable(executable: &Path) -> Option<PathBuf> {
    dunce::simplified(executable)
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
}

/// Finds the packaged extensions in the package at `package_root` and
/// takes the JDT LS files from `server`.
pub fn locate_launch_files(
    package_root: &Path,
    server: &ServerInstallation,
) -> Result<LaunchFiles, PluginError> {
    let lombok_agent = package_root.join(LOMBOK_AGENT);
    if !lombok_agent.is_file() {
        return Err(missing_resource("the Lombok agent"));
    }
    let extension_bundles = EXTENSION_BUNDLE_DIRECTORIES
        .iter()
        .flat_map(|directory| bundles_in(&package_root.join(directory)))
        .collect();
    Ok(LaunchFiles {
        launcher_jar: server.launcher_jar.clone(),
        lombok_agent,
        server_configuration: server.configuration.clone(),
        extension_bundles,
    })
}

/// The `-data` directory of the workspace.
pub fn data_directory(request: &LaunchRequest) -> PathBuf {
    request.workspace_state_directory.join(DATA_DIRECTORY)
}

/// The complete JDT LS command line after the `java` executable.
///
/// The plugin's own choices (heap, log level, the Equinox log in
/// `logDirectory`, `java.io.tmpdir` in `temporaryDirectory`) go before
/// `-jar <launcher>`; [`direct_java_arguments`] adds what JDT LS always
/// needs (the Lombok agent, `--add-opens`, the Equinox properties, the
/// tuning defaults), `-configuration <configuration_directory>` and
/// `-data <workspaceStateDirectory>/data`.
pub fn jdtls_arguments(
    request: &LaunchRequest,
    files: &LaunchFiles,
    configuration_directory: &Path,
) -> Vec<String> {
    let plugin_arguments = vec![
        format!("-Xmx{}m", request.heap_max_mb),
        format!("-Dlog.level={}", request.log_level),
        format!(
            "-Dosgi.logfile={}",
            request.log_directory.join(LOG_FILE).to_string_lossy()
        ),
        "-Declipse.log.include.commandline=false".to_string(),
        "-Declipse.log.size.max=1024".to_string(),
        "-Declipse.log.backup.max=2".to_string(),
        // No `-Declipse.log.level=WARNING`: JDT LS 1.61 reads the date of the
        // first Platform log event it saw when the client connects
        // (`LogHandler.setClientConnection` -> `LogReader.parseLogFile`), and
        // that event is its own INFO "is started" entry. Filtering INFO out
        // leaves the date null and the server dies with a
        // NullPointerException before it answers `initialize`. The client
        // mask `-Dlog.level` above is what limits what reaches the editor.
        format!(
            "-Djava.io.tmpdir={}",
            request.temporary_directory.to_string_lossy()
        ),
        "-jar".to_string(),
        files.launcher_jar.to_string_lossy().into_owned(),
    ];
    let resources = JdtDirectLaunchResources {
        launcher_jar_path: files.launcher_jar.clone(),
        configuration_directory: configuration_directory.to_path_buf(),
        lombok_agent_path: files.lombok_agent.clone(),
        java_debug_bundle_path: None,
    };
    direct_java_arguments(&plugin_arguments, &resources, &data_directory(request))
}

/// The gate that holds the backend's requests until JDT LS reports
/// `language/status` `ServiceReady`. Before that JDT LS answers project
/// commands and class file requests with errors or stale results.
pub fn service_ready_gate() -> Value {
    json!({
        "id": SERVICE_READY_GATE_ID,
        "until": "notification",
        "method": LANGUAGE_STATUS,
        "match": {"type": SERVICE_READY},
    })
}

/// Resolves the launch for `request` of the JDT LS installation `server`
/// with the extensions of the package at `package_root`.
///
/// `fragments` are the settings other plugins contributed for the
/// workspace, as the settings session last collected them. The function
/// prepares the configuration area under `stateDirectory`, which is the only
/// write it makes; a failed cleanup of older areas is reported through
/// `report` and does not fail the launch.
pub fn resolve(
    request: &LaunchRequest,
    server: &ServerInstallation,
    package_root: &Path,
    fragments: &[Value],
    trigger_files: &[String],
    now_unix_seconds: u64,
    report: &dyn Fn(&str, u64),
) -> Result<Value, PluginError> {
    check_server_runtime_for(request, server.minimum_java_version)?;
    let files = locate_launch_files(package_root, server)?;
    let area = prepare_jdt_configuration_area(
        &request.state_directory,
        &files.server_configuration,
        now_unix_seconds,
    )
    .map_err(|error| PluginError::new(LAUNCH_CONFIGURATION_FAILED, error.message))?;
    if area.cleanup_failure.is_some() {
        report(LAUNCH_CONFIGURATION_CLEANUP_FAILED, 1);
    }
    // Equinox also writes `../p2` next to the `-configuration` directory, so
    // the whole area is writable.
    let area_root = area
        .directory
        .parent()
        .map_or_else(|| area.directory.clone(), Path::to_path_buf);
    let mut initialization_options = adapt_initialization_options(
        SERVER_ID,
        None,
        &files.extension_bundles,
        jdt_settings(fragments, &request.java_runtimes),
    )
    .unwrap_or_else(|| json!({}));
    add_trigger_files(&mut initialization_options, trigger_files);
    let writable_paths: Vec<Value> = [
        area_root,
        data_directory(request),
        request.log_directory.clone(),
        request.temporary_directory.clone(),
    ]
    .iter()
    .map(|path| Value::String(path.to_string_lossy().into_owned()))
    .collect();
    Ok(json!({
        // The host resolves the tool from the user's choice and discovery.
        "executable": {
            "kind": "tool",
            "tool": SERVER_RUNTIME_TOOL,
        },
        "arguments": jdtls_arguments(request, &files, &area.directory),
        // JDT LS needs no variable beyond the ones the host passes.
        "environment": {},
        "initializationOptions": initialization_options,
        "workspaceConfiguration": java_configuration(fragments, &request.java_runtimes),
        "gates": [service_ready_gate()],
        "writablePaths": writable_paths,
    }))
}

/// The local paths of the workspace's folders, from the host's
/// `workspace/getFolders`. A remote workspace lists folders without a path,
/// which leaves the result empty.
fn workspace_folders(host: &Host, workspace: &WorkspaceContext) -> Result<Vec<PathBuf>, SdkError> {
    let answer = host.workspace().get_folders(workspace)?;
    Ok(answer
        .get("folders")
        .and_then(Value::as_array)
        .map(|folders| {
            folders
                .iter()
                .filter_map(|folder| folder.get("path").and_then(Value::as_str))
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default())
}

// A native DirEntry is intentionally classified only after the per-entry budget
// check. Tests can substitute entries and failures without changing permissions
// or relying on host-specific directory iteration behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TriggerKind {
    Directory,
    File,
    Other,
}

trait TriggerEntry {
    fn name(&self) -> OsString;
    fn path(&self) -> PathBuf;
    fn kind(&self) -> io::Result<TriggerKind>;
}

impl TriggerEntry for fs::DirEntry {
    fn name(&self) -> OsString {
        self.file_name()
    }

    fn path(&self) -> PathBuf {
        self.path()
    }

    fn kind(&self) -> io::Result<TriggerKind> {
        let kind = self.file_type()?;
        Ok(if kind.is_dir() {
            TriggerKind::Directory
        } else if kind.is_file() {
            TriggerKind::File
        } else {
            // In particular, do not follow links into a different source tree.
            TriggerKind::Other
        })
    }
}

#[derive(Default)]
struct TriggerScan {
    files: Vec<String>,
    unreadable: u64,
}

impl TriggerScan {
    fn note_error(&mut self, error: &io::Error) {
        // Optional source trees are allowed to disappear while the scan runs.
        if error.kind() != io::ErrorKind::NotFound {
            self.unreadable = self.unreadable.saturating_add(1);
        }
    }
}

fn native_trigger_entries(directory: &Path) -> io::Result<Vec<io::Result<fs::DirEntry>>> {
    Ok(fs::read_dir(directory)?.collect())
}

// Only this iterator owns the scan rules. It uses a substitute directory
// reader in tests to cover failed open, failed iteration and failed stat.
fn scan_trigger_files<E: TriggerEntry>(
    folders: &[PathBuf],
    max_entries: usize,
    mut read_directory: impl FnMut(&Path) -> io::Result<Vec<io::Result<E>>>,
) -> Result<TriggerScan, PluginError> {
    let mut scan = TriggerScan::default();
    let mut budget = max_entries;
    let mut roots: Vec<&PathBuf> = folders.iter().collect();
    roots.sort();
    roots.dedup();
    for root in roots {
        // Breadth first, so nearby sources precede deep source trees.
        let mut pending = VecDeque::from([(root.clone(), 0usize)]);
        while let Some((directory, depth)) = pending.pop_front() {
            if budget == 0 {
                return Ok(scan);
            }
            let entries = match read_directory(&directory) {
                Ok(entries) => entries,
                Err(error) => {
                    scan.note_error(&error);
                    continue;
                }
            };
            let mut sorted = Vec::new();
            for entry in entries {
                match entry {
                    Ok(entry) => sorted.push(entry),
                    Err(error) => scan.note_error(&error),
                }
            }
            sorted.sort_by_key(TriggerEntry::name);
            let mut subdirectories = Vec::new();
            for entry in sorted {
                if budget == 0 {
                    return Ok(scan);
                }
                budget -= 1;
                let name = entry.name();
                let Some(name) = name.to_str() else {
                    continue;
                };
                let kind = match entry.kind() {
                    Ok(kind) => kind,
                    Err(error) => {
                        scan.note_error(&error);
                        continue;
                    }
                };
                if kind == TriggerKind::Directory {
                    let skipped =
                        name.starts_with('.') || TRIGGER_SCAN_SKIPPED_DIRECTORIES.contains(&name);
                    if depth < TRIGGER_SCAN_DEPTH && !skipped {
                        subdirectories.push((entry.path(), depth + 1));
                    }
                } else if kind == TriggerKind::File
                    && name.ends_with(".java")
                    && !name.starts_with('.')
                {
                    scan.files.push(file_uri(&entry.path())?);
                    if scan.files.len() >= TRIGGER_FILE_LIMIT {
                        return Ok(scan);
                    }
                }
            }
            pending.extend(subdirectories);
        }
    }
    Ok(scan)
}

fn report_trigger_scan(scan: &TriggerScan, report: &dyn Fn(&str, u64)) {
    if scan.unreadable != 0 {
        report(LAUNCH_TRIGGER_SCAN_UNREADABLE, scan.unreadable);
    }
}

/// The `triggerFiles` of a launch: `file:` URIs of up to
/// [`TRIGGER_FILE_LIMIT`] `.java` files found under `folders`, folders and
/// entries in sorted order so the same workspace always names the same files.
/// The scan stops at [`TRIGGER_SCAN_DEPTH`] levels, skips hidden directories
/// and [`TRIGGER_SCAN_SKIPPED_DIRECTORIES`], does not follow symbolic links,
/// and reads at most [`TRIGGER_SCAN_ENTRY_BUDGET`] entries. Optional missing
/// folders contribute nothing; other I/O failures preserve partial results and
/// are counted once at the launch boundary without including file paths. Only
/// a launch reports diagnostic degradation, not this helper.
pub fn trigger_files(folders: &[PathBuf]) -> Result<Vec<String>, PluginError> {
    Ok(scan_trigger_files(folders, TRIGGER_SCAN_ENTRY_BUDGET, native_trigger_entries)?.files)
}

/// Seconds since the Unix epoch; a clock before 1970 counts as 0, which only
/// makes the configuration areas look older to the retention.
fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// Reports what detection could not do for the project; the launch stands.
pub fn report_model(model: &ProjectModel, report: &dyn Fn(&str, u64)) {
    if model.needs_servlet && model.servlet.runtime.is_none() {
        report(LAUNCH_SERVLET_CONTAINER_MISSING, 1);
    }
    if !model.unresolved.is_empty() {
        report(
            LAUNCH_PROJECT_MODEL_UNRESOLVED,
            model.unresolved.len() as u64,
        );
    }
    if model.truncated {
        report(LAUNCH_PROJECT_MODEL_TRUNCATED, 1);
    }
}

/// Runs a call of `java.resolveLaunch`.
///
/// The workspace comes from the call's `context`. The settings are the ones
/// the session last collected for it; on the first start of a workspace that
/// is only the plugin's own settings, and the session pushes the contributed
/// ones once the server runs.
pub fn resolve_launch_function(
    session: &SettingsSession,
    params: Value,
    context: &Context,
) -> Result<Value, FunctionError> {
    context.check_cancelled()?;
    let request = parse_launch_params(&params)?;
    check_server_runtime(&request)?;
    let home = request
        .server_home
        .as_deref()
        .ok_or_else(managed_runtimes::not_prepared)?;
    let server =
        managed_runtimes::inspect_server(home, &request.os, &request.architecture, &|| {
            context.check_cancelled().map_err(Into::into)
        })?
        .ok_or_else(managed_runtimes::incompatible_server)?;
    let package_root = package_root()?;
    let diagnostics = context.diagnostics();
    let operation = context.function_id().to_owned();
    // An error from `degraded` only means the arguments were invalid, so the
    // call's own result stands.
    let report = move |code: &str, count: u64| {
        let _ = diagnostics.degraded(&operation, code, Some(count));
    };
    let (fragments, folders) = match context.workspace_context() {
        Some(workspace) => {
            // The pushes after this start repeat the runtimes and the
            // project model settings it configures.
            session.set_java_runtimes(workspace, request.java_runtimes.clone());
            session.set_model_options(workspace, request.model_options.clone());
            let folders = match workspace_folders(context.host(), workspace) {
                Ok(folders) => folders,
                Err(_) => {
                    // The launch stands without the folders; JDT LS then lays
                    // out a plain workspace from the first opened file.
                    report(LAUNCH_WORKSPACE_FOLDERS_UNAVAILABLE, 1);
                    Vec::new()
                }
            };
            (session.start_fragments(workspace.context_id()), folders)
        }
        None => (Vec::new(), Vec::new()),
    };
    let scan = scan_trigger_files(&folders, TRIGGER_SCAN_ENTRY_BUDGET, native_trigger_entries)?;
    context.check_cancelled()?;
    report_trigger_scan(&scan, &report);
    let mut answer = resolve(
        &request,
        &server,
        &package_root,
        &fragments,
        &scan.files,
        now_unix_seconds(),
        &report,
    )?;
    if let Some(folder) = folders.first() {
        let (configuration, model) = crate::settings_session::with_native_model(
            answer["workspaceConfiguration"].clone(),
            folder,
            &request.model_options,
        )?;
        report_model(&model, &report);
        answer["workspaceConfiguration"] = configuration.clone();
        answer["initializationOptions"]["settings"] = configuration;
    }
    context.check_cancelled()?;
    if let Some(workspace) = context.workspace_context() {
        let jdk = &params["runtimes"]["jdk"];
        if let (Some(version), Some(home), Some(server_version)) = (
            jdk["version"].as_str(),
            jdk["home"].as_str(),
            params["runtimes"]["jdtls"]["version"].as_str(),
        ) {
            session.record_service_environment(
                workspace,
                params["settings"][SERVER_HOME_SETTING]
                    .as_str()
                    .unwrap_or(""),
                [
                    format!("JDT LS {server_version} · {}", server.home.display()),
                    format!("JDK {version} · {home}"),
                ],
            );
        }
    }
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jdtls;

    #[test]
    #[cfg(windows)]
    fn package_paths_do_not_break_java_agent_loading() {
        // Native Windows JDK 21 exits before initialize when -javaagent uses
        // canonicalize's verbatim prefix, even for a normal local package.
        let root =
            package_root_from_executable(Path::new(r"\\?\C:\plugins\Java 支持\bin\backend.exe"))
                .unwrap();
        assert_eq!(root, Path::new(r"C:\plugins\Java 支持"));
    }

    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    const LAUNCHER: &str = "plugins/org.eclipse.equinox.launcher_1.8.0.v20260804-1928.jar";

    /// A package laid out like the assembled one, a JDT LS installation like
    /// the user's, and the host's four directories, in a fresh directory that
    /// is removed on drop.
    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "lithe-java-launch-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ));
            let _ = fs::remove_dir_all(&root);
            let package = root.join("package");
            for directory in [
                "jdtls-extensions/lombok",
                "jdtls-extensions/java-debug",
                "jdtls-extensions/java-test/extensions",
            ] {
                fs::create_dir_all(package.join(directory)).unwrap();
            }
            for file in [
                "jdtls-extensions/lombok/lombok.jar",
                "jdtls-extensions/java-debug/com.microsoft.java.debug.plugin-0.53.0.jar",
                "jdtls-extensions/java-test/extensions/b.jar",
                "jdtls-extensions/java-test/extensions/a.jar",
                "jdtls-extensions/java-test/extensions/notes.txt",
            ] {
                fs::write(package.join(file), b"").unwrap();
            }
            let server = root.join("jdtls");
            for directory in ["plugins", "config_mac_arm"] {
                fs::create_dir_all(server.join(directory)).unwrap();
            }
            for file in [LAUNCHER, "plugins/org.eclipse.jdt.ls.core_1.50.0.jar"] {
                fs::write(server.join(file), b"").unwrap();
            }
            fs::write(
                server.join("config_mac_arm/config.ini"),
                b"osgi.bundles=example\n",
            )
            .unwrap();
            for directory in ["state", "workspace-state", "log", "tmp"] {
                fs::create_dir_all(root.join(directory)).unwrap();
            }
            Fixture { root }
        }

        fn package(&self) -> PathBuf {
            self.root.join("package")
        }

        fn server(&self) -> ServerInstallation {
            jdtls::inspect(&self.root.join("jdtls"), "config_mac_arm").unwrap()
        }

        fn params(&self) -> Value {
            json!({
                "serverID": "java",
                "platform": {"os": "macos", "arch": "arm64"},
                "stateDirectory": self.root.join("state"),
                "workspaceStateDirectory": self.root.join("workspace-state"),
                "logDirectory": self.root.join("log"),
                "temporaryDirectory": self.root.join("tmp"),
                "settings": {"heapMaxMB": 4096, "logLevel": "INFO"},
            })
        }

        fn request(&self) -> LaunchRequest {
            parse_launch_params(&self.params()).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn resolve_fixture(fixture: &Fixture, fragments: &[Value]) -> (Value, Vec<String>) {
        let reports = Mutex::new(Vec::new());
        let answer = {
            let report = |code: &str, _count: u64| reports.lock().unwrap().push(code.to_string());
            resolve(
                &fixture.request(),
                &fixture.server(),
                &fixture.package(),
                fragments,
                &[],
                1_000_000,
                &report,
            )
            .unwrap()
        };
        (answer, reports.into_inner().unwrap())
    }

    fn strings(value: &Value) -> Vec<String> {
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item.as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn the_params_are_read_with_the_declared_defaults() {
        let fixture = Fixture::new();
        let request = fixture.request();
        assert_eq!(request.os, "macos");
        assert_eq!(request.architecture, "arm64");
        assert_eq!(request.server_home, None);
        assert_eq!(request.server_runtime_version, None);
        assert_eq!(request.heap_max_mb, 4096);
        assert_eq!(request.log_level, "INFO");
        assert_eq!(request.state_directory, fixture.root.join("state"));

        let mut params = fixture.params();
        params["settings"] = json!({});
        let request = parse_launch_params(&params).unwrap();
        assert_eq!(request.heap_max_mb, DEFAULT_HEAP_MAX_MB);
        assert_eq!(request.log_level, DEFAULT_LOG_LEVEL);

        // An empty folder setting means "find JDT LS"; a folder is taken as is.
        params["settings"] = json!({"languageServerHome": ""});
        assert_eq!(parse_launch_params(&params).unwrap().server_home, None);
        let folder = fixture.root.join("jdtls");
        params["settings"] = json!({"languageServerHome": folder});
        assert_eq!(
            parse_launch_params(&params).unwrap().server_home,
            Some(folder)
        );
    }

    #[test]
    fn unusable_params_are_refused_with_one_code() {
        let fixture = Fixture::new();
        let edits: Vec<(&str, Value)> = vec![
            ("/serverID", json!("kotlin")),
            ("/platform/arch", json!("sparc")),
            ("/platform/os", json!("")),
            ("/platform/os", Value::Null),
            ("/stateDirectory", json!("relative/state")),
            ("/workspaceStateDirectory", Value::Null),
            ("/settings/heapMaxMB", json!(1024)),
            ("/settings/heapMaxMB", json!("4096")),
            ("/settings/logLevel", json!("DEBUG")),
        ];
        for (pointer, value) in edits {
            let mut params = fixture.params();
            *params.pointer_mut(pointer).unwrap() = value;
            let error = parse_launch_params(&params).unwrap_err();
            assert_eq!(error.code(), LAUNCH_INVALID_PARAMS, "{pointer}");
        }
        for folder in [json!("relative/jdtls"), json!(7)] {
            let mut params = fixture.params();
            params["settings"]["languageServerHome"] = folder;
            let error = parse_launch_params(&params).unwrap_err();
            assert_eq!(error.code(), LAUNCH_INVALID_PARAMS);
        }
        assert_eq!(
            parse_launch_params(&json!({})).unwrap_err().code(),
            LAUNCH_INVALID_PARAMS
        );
    }

    #[test]
    fn the_answer_runs_the_users_jdt_ls_on_the_runtime_tool() {
        let fixture = Fixture::new();
        let (answer, reports) = resolve_fixture(&fixture, &[]);
        assert!(reports.is_empty());
        assert_eq!(
            answer["executable"],
            json!({"kind": "tool", "tool": "jdtls-runtime"})
        );
        let arguments = strings(&answer["arguments"]);
        let package = fixture.package();
        let launcher = fixture.server().launcher_jar;
        let lombok = package.join("jdtls-extensions/lombok/lombok.jar");
        assert_eq!(
            arguments[0],
            format!("-javaagent:{}", lombok.to_string_lossy())
        );
        for expected in [
            "-Xms256m",
            "-Xmx4096m",
            "-Dlog.level=INFO",
            "--add-modules=ALL-SYSTEM",
            "--add-opens=java.base/java.util=ALL-UNNAMED",
            "--add-opens=java.base/java.lang=ALL-UNNAMED",
            "-Declipse.application=org.eclipse.jdt.ls.core.id1",
            "-Declipse.product=org.eclipse.jdt.ls.core.product",
            "-Declipse.log.size.max=1024",
        ] {
            assert!(
                arguments.iter().any(|argument| argument == expected),
                "{expected}"
            );
        }
        assert!(!arguments.iter().any(|argument| argument == "-Xmx2048m"));
        // The Equinox log filter would crash JDT LS 1.61 at `initialize`.
        assert!(!arguments
            .iter()
            .any(|argument| argument.starts_with("-Declipse.log.level=")));
        let log_file = fixture.root.join("log").join("jdt.log");
        assert!(arguments.contains(&format!("-Dosgi.logfile={}", log_file.to_string_lossy())));
        let temporary = fixture.root.join("tmp");
        assert!(arguments.contains(&format!("-Djava.io.tmpdir={}", temporary.to_string_lossy())));

        // `-jar <launcher> -configuration <area> ... -data <data>` closes the
        // line, and every JVM option comes before `-jar`.
        let jar = arguments
            .iter()
            .position(|argument| argument == "-jar")
            .unwrap();
        assert_eq!(Path::new(&arguments[jar + 1]), Path::new(&launcher));
        assert_eq!(arguments[jar + 2], "-configuration");
        let configuration = PathBuf::from(&arguments[jar + 3]);
        assert!(configuration.starts_with(fixture.root.join("state")));
        assert!(configuration.join("config.ini").is_file());
        assert!(!configuration.starts_with(&package));
        assert!(!configuration.starts_with(fixture.root.join("jdtls")));
        let data = fixture.root.join("workspace-state").join("data");
        assert_eq!(
            arguments[arguments.len() - 2..].to_vec(),
            vec!["-data".to_string(), data.to_string_lossy().into_owned()]
        );
        assert!(arguments[..jar]
            .iter()
            .all(|argument| argument.starts_with('-')));
    }

    #[test]
    fn the_answer_loads_the_bundles_and_waits_for_service_ready() {
        let fixture = Fixture::new();
        let (answer, _) = resolve_fixture(&fixture, &[]);
        let package = fixture.package();
        let expected: Vec<PathBuf> = [
            "jdtls-extensions/java-debug/com.microsoft.java.debug.plugin-0.53.0.jar",
            "jdtls-extensions/java-test/extensions/a.jar",
            "jdtls-extensions/java-test/extensions/b.jar",
        ]
        .iter()
        .map(|bundle| package.join(bundle))
        .collect();
        assert_eq!(
            strings(&answer["initializationOptions"]["bundles"])
                .into_iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            answer["initializationOptions"]["extendedClientCapabilities"]
                ["classFileContentsSupport"],
            json!(true)
        );
        assert_eq!(
            answer["gates"],
            json!([{
                "id": "service-ready",
                "until": "notification",
                "method": "language/status",
                "match": {"type": "ServiceReady"},
            }])
        );
        assert_eq!(answer["environment"], json!({}));
    }

    #[test]
    fn every_write_lies_in_a_host_directory() {
        let fixture = Fixture::new();
        let (answer, _) = resolve_fixture(&fixture, &[]);
        let writable = strings(&answer["writablePaths"]);
        assert_eq!(writable.len(), 4);
        let hosts: Vec<PathBuf> = ["state", "workspace-state", "log", "tmp"]
            .iter()
            .map(|directory| fixture.root.join(directory))
            .collect();
        for path in &writable {
            assert!(
                hosts.iter().any(|host| Path::new(path).starts_with(host)),
                "{path}"
            );
        }
        let arguments = strings(&answer["arguments"]);
        let configuration = arguments
            .windows(2)
            .find(|pair| pair[0] == "-configuration")
            .map(|pair| PathBuf::from(&pair[1]))
            .unwrap();
        assert!(writable
            .iter()
            .any(|path| configuration.starts_with(path) && Path::new(path).starts_with(&hosts[0])));
    }

    #[test]
    fn the_startup_settings_are_the_settings_a_push_sends() {
        let fixture = Fixture::new();
        let fragments = vec![json!({"java": {"import": {"maven": {"enabled": true}}}})];
        let (answer, _) = resolve_fixture(&fixture, &fragments);
        assert_eq!(
            answer["workspaceConfiguration"],
            java_configuration(&fragments, &[])
        );
        assert_eq!(
            answer["initializationOptions"]["settings"]["java"],
            answer["workspaceConfiguration"]["java"]
        );
        let (answer, _) = resolve_fixture(&fixture, &[]);
        assert_eq!(
            answer["workspaceConfiguration"],
            java_configuration(&[], &[])
        );
        assert!(answer
            .pointer("/initializationOptions/settings/java/configuration/runtimes")
            .is_none());
    }

    #[test]
    fn the_java_tool_the_host_resolved_is_the_default_runtime() {
        let fixture = Fixture::new();
        let mut params = fixture.params();
        params["tools"] = json!({
            "java": {"executable": "/jdk/17/bin/java", "home": "/jdk/17", "version": "17.0.19"}
        });
        let request = parse_launch_params(&params).unwrap();
        let expected = json!([{"name": "JavaSE-17", "path": "/jdk/17", "default": true}]);
        assert_eq!(
            request.java_runtimes,
            vec![JdtJavaRuntime {
                is_default: true,
                name: "JavaSE-17".to_string(),
                path: "/jdk/17".to_string(),
            }]
        );
        let report = |_: &str, _: u64| {};
        let answer = resolve(
            &request,
            &fixture.server(),
            &fixture.package(),
            &[],
            &[],
            1_000_000,
            &report,
        )
        .unwrap();
        assert_eq!(
            answer["initializationOptions"]["settings"]["java"]["configuration"]["runtimes"],
            expected
        );
        assert_eq!(
            answer["workspaceConfiguration"]["java"]["configuration"]["runtimes"],
            expected
        );

        // A tool chosen by executable has no home; a host that found nothing
        // sends no entry. Neither configures a runtime.
        for tools in [
            json!({"java": {"executable": "/usr/bin/java"}}),
            json!({"java": {"executable": "/usr/bin/java", "home": " ", "version": "17"}}),
            json!({"gradle": {"executable": "/g/bin/gradle", "home": "/g", "version": "8"}}),
            json!({}),
        ] {
            params["tools"] = tools;
            assert!(parse_launch_params(&params)
                .unwrap()
                .java_runtimes
                .is_empty());
        }
    }

    #[test]
    fn a_damaged_package_names_the_missing_file() {
        let fixture = Fixture::new();
        fs::remove_file(fixture.package().join("jdtls-extensions/lombok/lombok.jar")).unwrap();
        let error = locate_launch_files(&fixture.package(), &fixture.server()).unwrap_err();
        assert_eq!(error.code(), LAUNCH_RESOURCE_MISSING);
        assert_eq!(
            error.message(),
            missing_resource_message("the Lombok agent")
        );
    }

    #[test]
    fn a_runtime_older_than_java_25_is_refused_before_anything_is_written() {
        let fixture = Fixture::new();
        let mut params = fixture.params();
        for (version, accepted) in [
            ("17.0.19", false),
            ("8.0.392", false),
            ("21", false),
            ("21.0.9", false),
            ("25", true),
            ("25.0.5.0.2", true),
            ("26.0.2", true),
            ("unknown", true),
        ] {
            params["tools"] = json!({
                "jdtls-runtime": {"executable": "/jdk/bin/java", "home": "/jdk", "version": version}
            });
            let request = parse_launch_params(&params).unwrap();
            assert_eq!(
                check_server_runtime(&request).is_ok(),
                accepted,
                "{version}"
            );
            if !accepted {
                let report = |_: &str, _: u64| {};
                let error = resolve(
                    &request,
                    &fixture.server(),
                    &fixture.package(),
                    &[],
                    &[],
                    1_000_000,
                    &report,
                )
                .unwrap_err();
                assert_eq!(error.code(), LAUNCH_RUNTIME_TOO_OLD);
                assert_eq!(error.message(), RUNTIME_TOO_OLD_MESSAGE);
            }
        }
        assert!(fs::read_dir(fixture.root.join("state"))
            .unwrap()
            .next()
            .is_none());
        // Without a version the host's own resolution decides.
        params["tools"] = json!({});
        assert!(check_server_runtime(&parse_launch_params(&params).unwrap()).is_ok());

        // A JDT LS installation whose core bundles require Java 25 (such as
        // JDT LS 1.63+) refuses a Java 21 runtime with the exact required
        // version before writing anything to `state`.
        let mut server_25 = fixture.server();
        server_25.minimum_java_version = 25;
        params["tools"] = json!({
            "jdtls-runtime": {"executable": "/jdk21/bin/java", "home": "/jdk21", "version": "21.0.9"}
        });
        let request_21 = parse_launch_params(&params).unwrap();
        let report = |_: &str, _: u64| {};
        let error = resolve(
            &request_21,
            &server_25,
            &fixture.package(),
            &[],
            &[],
            1_000_000,
            &report,
        )
        .unwrap_err();
        assert_eq!(error.code(), LAUNCH_RUNTIME_TOO_OLD);
        assert_eq!(error.message(), runtime_too_old_message(25));
        assert!(error.message().contains("Java 25 or later"));
        assert!(fs::read_dir(fixture.root.join("state"))
            .unwrap()
            .next()
            .is_none());

        params["tools"] = json!({
            "jdtls-runtime": {"executable": "/jdk25/bin/java", "home": "/jdk25", "version": "25.0.4.1"}
        });
        let request_25 = parse_launch_params(&params).unwrap();
        assert!(resolve(
            &request_25,
            &server_25,
            &fixture.package(),
            &[],
            &[],
            1_000_000,
            &report,
        )
        .is_ok());
    }

    #[test]
    fn launch_codes_are_distinct_valid_plugin_codes() {
        let all: std::collections::BTreeSet<&str> = LAUNCH_ERROR_CODES
            .iter()
            .chain(LAUNCH_CATEGORIES)
            .copied()
            .collect();
        assert_eq!(
            all.len(),
            LAUNCH_ERROR_CODES.len() + LAUNCH_CATEGORIES.len()
        );
        assert!(all
            .iter()
            .all(|code| lithe_plugin_sdk::is_plugin_error_code(code)));
        assert!(lithe_plugin_sdk::is_function_id(LAUNCH_FUNCTION_ID));
    }

    // The injected reader models errors that cannot reliably be produced with
    // chmod on Windows or when a test process has elevated permissions.
    struct TriggerFixtureEntry {
        path: PathBuf,
        kind: Result<TriggerKind, io::ErrorKind>,
    }

    impl TriggerEntry for TriggerFixtureEntry {
        fn name(&self) -> OsString {
            self.path.file_name().unwrap().to_os_string()
        }

        fn path(&self) -> PathBuf {
            self.path.clone()
        }

        fn kind(&self) -> io::Result<TriggerKind> {
            self.kind.map_err(io::Error::from)
        }
    }

    #[test]
    fn trigger_scan_treats_disappeared_directories_as_optional_but_counts_io_denial() {
        let fixture = Fixture::new();
        let roots = vec![fixture.root.join("workspace")];
        let vanished =
            scan_trigger_files::<TriggerFixtureEntry>(&roots, TRIGGER_SCAN_ENTRY_BUDGET, |_| {
                Err(io::Error::from(io::ErrorKind::NotFound))
            })
            .unwrap();
        assert!(vanished.files.is_empty());
        assert_eq!(vanished.unreadable, 0);

        let denied =
            scan_trigger_files::<TriggerFixtureEntry>(&roots, TRIGGER_SCAN_ENTRY_BUDGET, |_| {
                Err(io::Error::from(io::ErrorKind::PermissionDenied))
            })
            .unwrap();
        assert!(denied.files.is_empty());
        assert_eq!(denied.unreadable, 1);
        let calls = std::cell::Cell::new(0);
        report_trigger_scan(&denied, &|code, count| {
            assert_eq!(code, LAUNCH_TRIGGER_SCAN_UNREADABLE);
            assert_eq!(count, 1);
            calls.set(calls.get() + 1);
        });
        assert_eq!(calls.get(), 1);
        report_trigger_scan(&vanished, &|_, _| panic!("missing folders are normal"));

        let disappeared_entries =
            scan_trigger_files::<TriggerFixtureEntry>(&roots, TRIGGER_SCAN_ENTRY_BUDGET, |_| {
                Ok(vec![
                    Err(io::Error::from(io::ErrorKind::NotFound)),
                    Ok(TriggerFixtureEntry {
                        path: roots[0].join("Gone.java"),
                        kind: Err(io::ErrorKind::NotFound),
                    }),
                ])
            })
            .unwrap();
        assert!(disappeared_entries.files.is_empty());
        assert_eq!(disappeared_entries.unreadable, 0);
    }

    #[test]
    fn trigger_scan_retains_partial_results_and_reports_one_redacted_summary() {
        let fixture = Fixture::new();
        let root = fixture.root.join("workspace");
        let java = root.join("Main.java");
        let unreadable = root.join("Unreadable.java");
        let child = root.join("src");
        let scan = scan_trigger_files::<TriggerFixtureEntry>(
            std::slice::from_ref(&root),
            TRIGGER_SCAN_ENTRY_BUDGET,
            |directory| {
                if directory == root {
                    return Ok(vec![
                        Ok(TriggerFixtureEntry {
                            path: child.clone(),
                            kind: Ok(TriggerKind::Directory),
                        }),
                        Ok(TriggerFixtureEntry {
                            path: java.clone(),
                            kind: Ok(TriggerKind::File),
                        }),
                        Err(io::Error::from(io::ErrorKind::PermissionDenied)),
                        Ok(TriggerFixtureEntry {
                            path: unreadable.clone(),
                            kind: Err(io::ErrorKind::PermissionDenied),
                        }),
                        Ok(TriggerFixtureEntry {
                            path: root.join("Link.java"),
                            kind: Ok(TriggerKind::Other),
                        }),
                    ]);
                }
                Err(io::Error::from(io::ErrorKind::PermissionDenied))
            },
        )
        .unwrap();
        assert_eq!(scan.files, vec![file_uri(&java).unwrap()]);
        assert_eq!(scan.unreadable, 3);
        let calls = std::cell::Cell::new(0);
        report_trigger_scan(&scan, &|code, count| {
            assert_eq!(code, LAUNCH_TRIGGER_SCAN_UNREADABLE);
            assert_eq!(count, 3);
            calls.set(calls.get() + 1);
        });
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn trigger_scan_keeps_the_entry_budget_when_reading_fixture_directories() {
        let fixture = Fixture::new();
        let root = fixture.root.join("workspace");
        let first = root.join("A.java");
        let second = root.join("B.java");
        let third = root.join("C.java");
        let scan =
            scan_trigger_files::<TriggerFixtureEntry>(std::slice::from_ref(&root), 2, |_| {
                Ok(vec![
                    Ok(TriggerFixtureEntry {
                        path: third.clone(),
                        kind: Ok(TriggerKind::File),
                    }),
                    Ok(TriggerFixtureEntry {
                        path: first.clone(),
                        kind: Ok(TriggerKind::File),
                    }),
                    Ok(TriggerFixtureEntry {
                        path: second.clone(),
                        kind: Ok(TriggerKind::File),
                    }),
                ])
            })
            .unwrap();
        assert_eq!(
            scan.files,
            vec![file_uri(&first).unwrap(), file_uri(&second).unwrap()]
        );
        assert_eq!(scan.unreadable, 0);
    }

    #[test]
    fn trigger_files_name_java_files_in_order_and_skip_hidden_and_build_trees() {
        let fixture = Fixture::new();
        let workspace = fixture.root.join("workspace");
        for directory in [
            "src/demo",
            "src/.hidden",
            "target/classes",
            "node_modules/x",
            "lib",
            "a/b/c/d/e/f/g/h/i",
        ] {
            fs::create_dir_all(workspace.join(directory)).unwrap();
        }
        for file in [
            "src/demo/Main.java",
            "src/demo/notes.txt",
            "src/.hidden/Hidden.java",
            "target/classes/Generated.java",
            "node_modules/x/Dep.java",
            "lib/Util.java",
            "lib/.Dot.java",
            "a/b/c/d/e/f/g/h/i/Deep.java",
        ] {
            fs::write(workspace.join(file), b"").unwrap();
        }
        let files = trigger_files(&[workspace.clone(), workspace.clone()]).unwrap();
        assert_eq!(
            files,
            vec![
                file_uri(&workspace.join("lib/Util.java")).unwrap(),
                file_uri(&workspace.join("src/demo/Main.java")).unwrap(),
            ]
        );
        assert!(trigger_files(&[fixture.root.join("missing")])
            .unwrap()
            .is_empty());
        assert!(trigger_files(&[]).unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn trigger_scan_does_not_follow_symlinks_or_report_them_unreadable() {
        let fixture = Fixture::new();
        let workspace = fixture.root.join("workspace");
        let outside = fixture.root.join("outside");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let real = workspace.join("Real.java");
        let outside_file = outside.join("Outside.java");
        fs::write(&real, b"").unwrap();
        fs::write(&outside_file, b"").unwrap();
        std::os::unix::fs::symlink(&outside_file, workspace.join("Link.java")).unwrap();
        std::os::unix::fs::symlink(&outside, workspace.join("linked_dir")).unwrap();

        let scan = scan_trigger_files(
            std::slice::from_ref(&workspace),
            TRIGGER_SCAN_ENTRY_BUDGET,
            native_trigger_entries,
        )
        .unwrap();
        assert_eq!(scan.files, vec![file_uri(&real).unwrap()]);
        assert_eq!(scan.unreadable, 0);
    }

    #[test]
    fn trigger_files_stop_at_the_limit() {
        let fixture = Fixture::new();
        let workspace = fixture.root.join("many");
        fs::create_dir_all(&workspace).unwrap();
        for index in 0..(TRIGGER_FILE_LIMIT + 5) {
            fs::write(workspace.join(format!("C{index:03}.java")), b"").unwrap();
        }
        assert_eq!(
            trigger_files(&[workspace]).unwrap().len(),
            TRIGGER_FILE_LIMIT
        );
    }

    #[test]
    fn resolve_passes_trigger_files_to_the_initialization_options() {
        let fixture = Fixture::new();
        let report = |_: &str, _: u64| {};
        let trigger = vec!["file:///w/src/demo/Main.java".to_string()];
        let answer = resolve(
            &fixture.request(),
            &fixture.server(),
            &fixture.package(),
            &[],
            &trigger,
            1_000_000,
            &report,
        )
        .unwrap();
        assert_eq!(
            answer["initializationOptions"]["triggerFiles"],
            json!(["file:///w/src/demo/Main.java"])
        );
        let (answer, _) = resolve_fixture(&fixture, &[]);
        assert!(answer["initializationOptions"]
            .get("triggerFiles")
            .is_none());
    }
}

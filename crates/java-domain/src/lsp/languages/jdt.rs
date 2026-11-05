//! JDT LS-specific policy kept outside the generic LSP client and transport.
//!
//! The adapter is deliberately pure: it describes launch arguments, provider
//! notifications, configuration responses, and virtual-source requests. The
//! process engine remains responsible for creating directories and performing
//! all I/O.

#![allow(dead_code)] // This module is an engine adapter seam; integration is intentionally separate.

use crate::lsp::JavaRuntimeCandidate;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use url::Url;

const JAVA_PROVIDER_ID: &str = "java";
const JDT_URI_SCHEME: &str = "jdt";
const JDTLS_DATA_DIRECTORY: &str = "jdtls";
const JAVA_DECOMPILE_COMMAND: &str = "java.decompile";
const DID_CHANGE_CONFIGURATION_METHOD: &str = "workspace/didChangeConfiguration";
/// JDT LS extension that force-updates the listed projects' build configuration.
const PROJECT_CONFIGURATIONS_UPDATE_METHOD: &str = "java/projectConfigurationsUpdate";
const LANGUAGE_STATUS_METHOD: &str = "language/status";
const WORK_DONE_PROGRESS_METHOD: &str = "$/progress";
const SERVICE_READY_STATUS: &str = "ServiceReady";
const ERROR_STATUS: &str = "Error";
/// JVM system property read by JDT LS's `org.eclipse.jdt.ls.filesystem` bundle.
///
/// When the property is absent, JDT LS writes `.project`, `.classpath`,
/// `.factorypath`, and `.settings/*.prefs` into every imported module directory.
/// `false` redirects them to the `-data` metadata area, so opening a project
/// never adds Eclipse files to the user's tree. Files that already exist at a
/// module root still take precedence; `jdt_project_metadata` removes those.
const METADATA_AT_PROJECT_ROOT_PROPERTY: &str = "-Djava.import.generatesMetadataFilesAtProjectRoot";
/// Heap ceiling passed to the JDT LS JVM on both launch paths.
///
/// JDT LS switches to a batched, memory-frugal Maven import as soon as its
/// maximum heap is at most 1.5 GB. That path hands a `null` project to
/// `IProject.open` when several modules share `groupId` and `artifactId`, which
/// fails the whole Java service instead of the affected modules. Keep the
/// ceiling above that threshold, and at the 2 GB upstream documents for the
/// language server, so a normal multi-module workspace imports through the
/// regular path.
const JDTLS_MAX_HEAP_ARGUMENT: &str = "-Xmx2048m";

/// JDT LS readiness transition conveyed through its `language/status`
/// extension after the standard LSP initialize handshake.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum JdtReadinessSignal {
    /// Project import and Java service initialization have completed.
    Ready,
    /// JDT LS reported that its Java service could not become usable.
    Failed(String),
}

/// Maven artifact transfer details extracted from JDT LS work-done progress.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct JdtDownloadProgress {
    /// File name only; repository URLs can contain credentials or private hosts.
    pub artifact: String,
    /// Repository host without path, query, or credentials.
    pub repository_host: Option<String>,
    /// Bytes transferred according to JDT LS, when its progress includes a size pair.
    pub downloaded_bytes: Option<u64>,
    /// Expected artifact size according to JDT LS, when the server reports it.
    pub total_bytes: Option<u64>,
}

/// Observable Java workspace-import activity derived from standard progress events.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct JdtImportProgress {
    /// Stable signature used to ignore duplicate progress notifications.
    pub activity_signature: String,
    /// JDT LS work phase such as Maven import or classpath setup.
    pub phase: Option<String>,
    /// Outer workspace-import percentage, which can stay fixed during downloads.
    pub percentage: Option<u64>,
    /// Current Maven project when JDT LS includes it in the progress message.
    pub current_project: Option<String>,
    /// Current artifact transfer, if the progress message contains a repository URL.
    pub download: Option<JdtDownloadProgress>,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JdtStartContext {
    pub provider_id: String,
    pub workspace_root: PathBuf,
    /// Engine-owned cache/state root. The adapter only derives a child path.
    pub data_root: PathBuf,
    #[serde(default)]
    pub selected_java_executable: Option<PathBuf>,
    #[serde(default)]
    pub direct_launch_resources: Option<JdtDirectLaunchResources>,
    #[serde(default)]
    pub arguments: Vec<String>,
    /// Opaque platform-provided digest of the build-system inputs that decide
    /// how JDT LS will import the workspace: root build files and the set of
    /// module directories, plus the bundled JDT LS version.
    ///
    /// JDT LS never invalidates its own `-data` state, so a workspace that
    /// gains or loses a module keeps resolving against a stale project model
    /// and silently loses cross-module navigation. Mixing this digest into the
    /// state-directory name gives structurally different workspaces different
    /// directories while keeping the previous one intact for a later switch
    /// back. `None` preserves the pre-fingerprint directory name so existing
    /// caches stay valid.
    #[serde(default)]
    pub workspace_fingerprint: Option<String>,
}

/// JDT LS settings keys that other plugins may contribute to this plugin's
/// language server.
///
/// JDT LS imports Maven and Gradle projects itself, but the values it imports
/// with (settings files, local repository, offline mode, Gradle user home,
/// source folders read from build files) belong to the build-system plugins
/// that own those projects. This plugin only declares which part of JDT LS's
/// own settings namespace it accepts from them; it never computes those
/// values. An entry ending in `.` accepts every key below it; any other entry
/// accepts exactly that key.
///
/// `java.project.sourcePaths` is listed because Rust Core sent the source
/// folders it read from Maven POMs under that key.
// The contributions arrive through cross-plugin function calls that the host
// relays (this plugin, Core, the build-system plugin). The interface is in
// `jdt_contributions`, the calls are in the backend's `plugin_functions`
// module, and the backend's `settings_session` module sends the merged result
// to JDT LS. When the host reports that JDT LS is running, the backend asks
// every enabled provider of `languageImport.settings` and passes the answers
// in `JdtSettings::contributed`. Later changes work the other way round: the
// build-system plugin calls the change function this plugin exports and
// passes the complete new values in the call. In both cases the backend sends
// them as `workspace/didChangeConfiguration` through the host's
// language-server channel.
//
// JDT LS has no settings key for Maven profiles. Rust Core applied them per
// project with the `java.project.updateSettings` command and
// `org.eclipse.m2e.core.selectedProfiles`, which a settings fragment cannot
// express. The Maven plugin computes the profiles, so the same change
// function carries a per-project list, and the backend sends one
// `workspace/executeCommand` per project after the settings (see
// `settings_session`).
pub const CONTRIBUTED_SETTINGS_KEYS: &[&str] = &[
    "java.configuration.maven.",
    "java.import.maven.",
    "java.import.gradle.",
    "java.project.sourcePaths",
];

/// One contributed settings fragment, split into dotted leaf keys.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ContributedSettings {
    /// Leaves inside [`CONTRIBUTED_SETTINGS_KEYS`], such as
    /// `java.import.gradle.offline.enabled`, in key order.
    pub accepted: BTreeMap<String, Value>,
    /// Leaf keys outside [`CONTRIBUTED_SETTINGS_KEYS`]; they never reach
    /// JDT LS.
    pub rejected: Vec<String>,
}

/// Splits a contributed fragment into the leaves this plugin accepts and the
/// ones it drops.
///
/// A fragment may nest objects (`{"java": {"import": {...}}}`), use dotted
/// keys (`{"java.import.gradle.offline.enabled": true}`) or mix both: objects
/// are walked down to their leaves, and arrays and scalars are leaves. A key
/// with an empty segment is rejected. A fragment that is not an object
/// contributes nothing. The host checks the declared keys before it forwards
/// a fragment; this check repeats it so a value outside them can never reach
/// JDT LS.
pub fn accept_contributed_settings(fragment: &Value) -> ContributedSettings {
    let mut contributed = ContributedSettings::default();
    if let Value::Object(entries) = fragment {
        collect_contributed_leaves(entries, "", &mut contributed);
    }
    contributed
}

fn collect_contributed_leaves(
    entries: &Map<String, Value>,
    prefix: &str,
    contributed: &mut ContributedSettings,
) {
    for (key, value) in entries {
        let key = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match value {
            Value::Object(children) => collect_contributed_leaves(children, &key, contributed),
            leaf if is_contributable_key(&key) => {
                contributed.accepted.insert(key, leaf.clone());
            }
            _ => contributed.rejected.push(key),
        }
    }
}

fn is_contributable_key(key: &str) -> bool {
    !key.split('.').any(str::is_empty)
        && CONTRIBUTED_SETTINGS_KEYS
            .iter()
            .any(|accepted| match accepted.strip_suffix('.') {
                Some(_) => key.starts_with(accepted),
                None => key == *accepted,
            })
}

/// Where the settings contributed by associated plugins stand for the
/// current session.
///
/// Rust Core tracked the Maven profiles it applied after the import with a
/// richer task status; the plugin only needs to know whether a contribution
/// is still arriving and whether one failed.
// `jdt_contributions::Contributions::status` derives `Settled` and `Failed`
// from the provider calls: a call that failed or timed out marks its provider
// as failed, and the user-visible message names that plugin.
//
// `Pending` is not derived by `Contributions`. The backend's `settings_session`
// module holds the contributions only after every provider call has ended, so
// nothing reports `Pending` today; it stays for a status surface that wants
// to show a collection in progress.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ContributedSettingsStatus {
    /// Every associated plugin answered, or none is associated.
    #[default]
    Settled,
    /// The host is still collecting or applying a contribution.
    Pending,
    /// An associated plugin failed or timed out; the session runs with the
    /// settings that did arrive.
    Failed,
}

/// A JDK JDT LS may bind a project to, keyed by its execution environment.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct JdtJavaRuntime {
    /// Selected project VM; at most one runtime is the default.
    pub is_default: bool,
    /// Execution-environment name such as `JavaSE-25` or `JavaSE-1.8`; JDT LS
    /// matches it against the release a project compiles for.
    pub name: String,
    /// JDK home directory.
    pub path: String,
}

/// Values Lithe owns in JDT LS's `settings.java`, borrowed from one session.
#[derive(Debug, Clone, Copy, Default)]
pub struct JdtSettings<'a> {
    /// Workspace indexing policy, when the host has installed one.
    pub indexing_root_uri: Option<&'a str>,
    /// Plugin-declared completion options from this session's initialization;
    /// all leaves survive except the provider-owned guessMethodArguments.
    pub completion: Option<&'a Value>,
    /// Original plugin import exclusions, before Lithe overlays the workspace policy.
    pub import_exclusions: Option<&'a Value>,
    /// Settings fragments from associated plugins, applied in order; only
    /// leaves inside [`CONTRIBUTED_SETTINGS_KEYS`] are used. Empty for a
    /// plain Java workspace, which JDT LS opens without an import step.
    pub contributed: &'a [Value],
    /// JDKs installed on this machine, at most one per execution environment.
    pub java_runtimes: &'a [JdtJavaRuntime],
}

/// Picks one JDK per Java major version for `java.configuration.runtimes`.
///
/// Without these, JDT LS knows only the JDK it runs on, so a project that
/// compiles for a newer release fails with "release N is not found in the
/// system" even though that JDK is installed. The first valid default wins its
/// major; otherwise, the first JDK listed for that major wins. Platforms order
/// candidates by preference. Versions that do not parse are skipped, not guessed.
pub fn jdt_java_runtimes(candidates: &[JavaRuntimeCandidate]) -> Vec<JdtJavaRuntime> {
    let selected = candidates.iter().find(|candidate| {
        candidate.is_default
            && !candidate.home_path.trim().is_empty()
            && java_major_version(&candidate.version).is_some()
    });
    let mut by_major = BTreeMap::new();
    // The explicit selection wins even over another installation of the same major.
    for candidate in selected.into_iter().chain(candidates) {
        let Some(major) = java_major_version(&candidate.version) else {
            continue;
        };
        if candidate.home_path.trim().is_empty() {
            continue;
        }
        by_major.entry(major).or_insert(candidate);
    }
    by_major
        .into_iter()
        .map(|(major, candidate)| JdtJavaRuntime {
            name: if major == 8 {
                "JavaSE-1.8".to_string()
            } else {
                format!("JavaSE-{major}")
            },
            path: candidate.home_path.trim().to_string(),
            is_default: selected.is_some_and(|chosen| std::ptr::eq(chosen, candidate)),
        })
        .collect()
}

/// Major version of a `java -version` string: `25.0.4` → 25, `1.8.0_402` → 8.
/// Versions before 8 have no JDT execution environment Lithe supports.
fn java_major_version(version: &str) -> Option<u32> {
    let mut parts = version
        .trim()
        .split(|character: char| !character.is_ascii_digit());
    let first = parts.next()?.parse::<u32>().ok()?;
    let major = if first == 1 {
        parts.next()?.parse::<u32>().ok()?
    } else {
        first
    };
    (major >= 8).then_some(major)
}

/// Platform-resolved files required to launch JDT LS without a shell wrapper.
#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JdtDirectLaunchResources {
    pub launcher_jar_path: PathBuf,
    /// Writable Equinox configuration area passed as `-configuration`. The
    /// engine supplies the cache copy from `jdt_configuration`, never the
    /// packaged directory, because Equinox writes its state here.
    pub configuration_directory: PathBuf,
    pub lombok_agent_path: PathBuf,
    #[serde(default)]
    pub java_debug_bundle_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
/// Request for the deterministic directory key used by JDT LS state.
pub struct JdtWorkspaceKeyRequest {
    /// Absolute platform workspace path used only to derive a stable local key.
    pub workspace_root: String,
    /// Optional platform-computed build-structure digest mixed into the key.
    #[serde(default)]
    pub workspace_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
/// Stable key returned to platform cache-maintenance adapters.
pub struct JdtWorkspaceKeyResponse {
    /// Lowercase SHA-256 key naming the workspace's JDT LS state directory.
    pub workspace_key: String,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JdtStartAdaptation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable: Option<PathBuf>,
    pub arguments: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_directory: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceConfigurationItem {
    #[serde(default)]
    pub scope_uri: Option<String>,
    #[serde(default)]
    pub section: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderNotification {
    pub method: String,
    pub params: Value,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderLocation {
    pub uri: String,
    #[serde(default)]
    pub is_read_only: bool,
    #[serde(default)]
    pub display_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteCommandParams {
    pub command: String,
    pub arguments: Vec<Value>,
}

/// Applies the JDT LS-owned part of a provider start plan.
///
/// `data_directory` is returned to the engine as a directory requirement; this
/// function never creates it. Arguments owned by this adapter are replaced so
/// repeated adaptation is deterministic and cannot leave two `-data` roots.
pub fn adapt_start(context: &JdtStartContext) -> JdtStartAdaptation {
    if !is_java_provider(&context.provider_id) {
        return JdtStartAdaptation {
            executable: None,
            arguments: context.arguments.clone(),
            data_directory: None,
        };
    }

    let data_directory = context
        .data_root
        .join(JDTLS_DATA_DIRECTORY)
        .join(workspace_key(
            &context.workspace_root,
            context.workspace_fingerprint.as_deref(),
        ));
    let (executable, arguments) = match (
        &context.selected_java_executable,
        &context.direct_launch_resources,
    ) {
        (Some(java_executable), Some(resources)) => (
            Some(java_executable.clone()),
            direct_java_arguments(&context.arguments, resources, &data_directory),
        ),
        _ => (
            None,
            wrapper_arguments(
                &context.arguments,
                context.selected_java_executable.as_deref(),
                &data_directory,
            ),
        ),
    };

    JdtStartAdaptation {
        executable,
        arguments,
        data_directory: Some(data_directory),
    }
}

/// Adds the JDT LS client extensions required for Java tooling.
///
/// Catalog-provided options are preserved, while the provider-owned capability
/// is authoritative because virtual class files cannot be opened without it.
/// Extension bundles are appended in caller order without duplicating catalog
/// entries, which keeps Debug and Test plugin activation deterministic.
pub fn adapt_initialization_options(
    provider_id: &str,
    initialization_options: Option<Value>,
    java_extension_bundle_paths: &[PathBuf],
    settings: JdtSettings<'_>,
) -> Option<Value> {
    if !is_java_provider(provider_id) {
        return initialization_options;
    }

    let mut options = match initialization_options {
        Some(Value::Object(options)) => options,
        _ => Map::new(),
    };
    let extended = options
        .entry("extendedClientCapabilities")
        .or_insert_with(|| json!({}));
    if !extended.is_object() {
        *extended = json!({});
    }
    extended
        .as_object_mut()
        .expect("the extended capabilities were normalized to an object")
        .insert("classFileContentsSupport".to_string(), Value::Bool(true));

    if !java_extension_bundle_paths.is_empty() {
        let bundles = options
            .entry("bundles")
            .or_insert_with(|| Value::Array(Vec::new()));
        if !bundles.is_array() {
            *bundles = Value::Array(Vec::new());
        }
        let bundles = bundles
            .as_array_mut()
            .expect("the Java extension bundles were normalized to an array");
        for bundle_path in java_extension_bundle_paths {
            let bundle = Value::String(bundle_path.to_string_lossy().into_owned());
            if !bundles.contains(&bundle) {
                bundles.push(bundle);
            }
        }
    }

    // JDT LS reads its build-import settings from `initializationOptions`
    // while it handles `initialize`, and the project import starts right
    // after. Contributed values sent only through `didChangeConfiguration`
    // would arrive after that import already resolved the projects with JDT
    // LS's defaults, so they must be present here as well.
    merge_java_settings(&mut options, java_settings(settings));

    Some(Value::Object(options))
}

/// Overlays the provider-owned `settings.java` values onto catalog-provided
/// initialization options.
///
/// Catalog keys that the provider does not own are preserved, while the
/// build, completion, code-lens and contributed values stay authoritative.
fn merge_java_settings(options: &mut Map<String, Value>, provider_settings: Value) {
    let settings = options.entry("settings").or_insert_with(|| json!({}));
    if !settings.is_object() {
        *settings = json!({});
    }
    let settings = settings
        .as_object_mut()
        .expect("the settings value was normalized to an object");
    let java = settings.entry("java").or_insert_with(|| json!({}));
    if !java.is_object() {
        *java = json!({});
    }
    let java = java
        .as_object_mut()
        .expect("the java settings were normalized to an object");
    let Some(provider_java) = provider_settings
        .get("java")
        .and_then(Value::as_object)
        .cloned()
    else {
        return;
    };
    for (key, value) in provider_java {
        if key == "completion" {
            let completion = java.entry(key).or_insert_with(|| json!({}));
            if !completion.is_object() {
                *completion = json!({});
            }
            completion.as_object_mut().unwrap().extend(
                value
                    .as_object()
                    .expect("completion settings are an object")
                    .clone(),
            );
            continue;
        }
        // `java.import` holds both values the provider owns (the exclusions
        // and the contributed `maven`/`gradle` leaves) and options a catalog
        // may declare that nobody else sets (`gradle.version`,
        // `gradle.java.home`, ...). The owned values merge into the declared
        // object instead of replacing it, and the exclusions are combined.
        if key == "import" {
            let import = java
                .entry("import".to_string())
                .or_insert_with(|| json!({}));
            if !import.is_object() {
                *import = json!({});
            }
            let import = import
                .as_object_mut()
                .expect("the import value was normalized to an object");
            let Value::Object(owned) = value else {
                continue;
            };
            for (import_key, import_value) in owned {
                if import_key == "exclusions" {
                    let mut combined = import
                        .get("exclusions")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    for exclusion in import_value.as_array().into_iter().flatten() {
                        if !combined.contains(exclusion) {
                            combined.push(exclusion.clone());
                        }
                    }
                    import.insert(import_key, json!(combined));
                } else {
                    merge_value(
                        import.entry(import_key).or_insert(Value::Null),
                        import_value,
                    );
                }
            }
            continue;
        }
        java.insert(key, value);
    }
}

/// Merges `value` into `target`: objects merge key by key, and any other
/// value replaces what `target` held.
fn merge_value(target: &mut Value, value: Value) {
    match (target, value) {
        (Value::Object(target), Value::Object(value)) => {
            for (key, child) in value {
                merge_value(target.entry(key).or_insert(Value::Null), child);
            }
        }
        (target, value) => *target = value,
    }
}

/// Returns JDT LS configuration values in the same order as the requested
/// `workspace/configuration` items. `None` delegates non-Java providers to the
/// generic engine behavior.
pub fn workspace_configuration(
    provider_id: &str,
    items: &[WorkspaceConfigurationItem],
    settings: JdtSettings<'_>,
) -> Option<Vec<Value>> {
    if !is_java_provider(provider_id) {
        return None;
    }
    Some(
        items
            .iter()
            .map(|item| java_configuration_for_section(item.section.as_deref(), settings))
            .collect(),
    )
}

/// Notification the engine sends after the generic `initialized` handshake.
pub fn initialized_notification(
    provider_id: &str,
    settings: JdtSettings<'_>,
) -> Option<ProviderNotification> {
    is_java_provider(provider_id).then(|| settings_notification(settings))
}

/// `workspace/didChangeConfiguration` carrying every Java setting Lithe owns,
/// including the contributed ones.
///
/// When a contributed Maven settings path differs from the one JDT LS holds,
/// JDT LS reloads it and force-updates every Maven project itself
/// (`StandardPreferenceManager.update`), including an import still in
/// progress. The backend sends this notification again whenever a
/// build-system plugin reports that its contribution changed.
pub fn settings_notification(settings: JdtSettings<'_>) -> ProviderNotification {
    ProviderNotification {
        method: DID_CHANGE_CONFIGURATION_METHOD.to_string(),
        params: json!({
            "settings": java_settings(settings)
        }),
    }
}

/// Forces JDT LS to re-resolve the given projects even though neither their
/// build files nor the contributed settings changed.
///
/// A JDT LS restart does not do this: its importer skips projects whose build
/// file is older than the saved workspace state, so dependencies that failed
/// to resolve stay missing. `java/projectConfigurationsUpdate` is the
/// upstream "update project" action and ignores that check. The URIs are the
/// projects JDT LS itself reports (`java.project.getAll`); repeated URIs are
/// sent once, in first-seen order.
pub fn project_update_notification(project_uris: &[String]) -> Option<ProviderNotification> {
    let mut seen = std::collections::BTreeSet::new();
    let identifiers = project_uris
        .iter()
        .filter(|uri| seen.insert(uri.as_str()))
        .map(|uri| json!({ "uri": uri }))
        .collect::<Vec<_>>();
    (!identifiers.is_empty()).then(|| ProviderNotification {
        method: PROJECT_CONFIGURATIONS_UPDATE_METHOD.to_string(),
        params: json!({ "identifiers": identifiers }),
    })
}

/// Returns whether the provider has a readiness phase after standard LSP
/// initialization. JDT LS cannot safely serve semantic requests until its Java
/// project import publishes `ServiceReady`.
pub fn waits_for_service_ready(provider_id: &str) -> bool {
    is_java_provider(provider_id)
}

/// Reduces one JDT LS status notification to the readiness transition owned by
/// the generic process engine. Other providers and informational statuses do
/// not affect lifecycle state.
pub fn readiness_signal(
    provider_id: &str,
    method: Option<&str>,
    params: Option<&Value>,
) -> Option<JdtReadinessSignal> {
    if !is_java_provider(provider_id) || method != Some(LANGUAGE_STATUS_METHOD) {
        return None;
    }
    let params = params?.as_object()?;
    match params.get("type").and_then(Value::as_str) {
        Some(SERVICE_READY_STATUS) => Some(JdtReadinessSignal::Ready),
        Some(ERROR_STATUS) => Some(JdtReadinessSignal::Failed(
            params
                .get("message")
                .and_then(Value::as_str)
                .filter(|message| !message.trim().is_empty())
                .unwrap_or("JDT LS reported an unknown Java service error.")
                .to_string(),
        )),
        _ => None,
    }
}

/// Extracts observability-only Maven import details from JDT LS `$/progress`.
///
/// Lifecycle correctness must never depend on this parser because progress text
/// is owned by JDT LS and can change between releases. Readiness continues to
/// depend exclusively on `language/status: ServiceReady`.
pub fn import_progress(
    provider_id: &str,
    method: Option<&str>,
    params: Option<&Value>,
) -> Option<JdtImportProgress> {
    if !is_java_provider(provider_id) || method != Some(WORK_DONE_PROGRESS_METHOD) {
        return None;
    }
    let value = params?.get("value")?;
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("report");
    let message = value
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| value.get("title").and_then(Value::as_str))
        .unwrap_or("")
        .trim();
    let percentage = value.get("percentage").and_then(Value::as_u64);
    if message.is_empty() && percentage.is_none() {
        return None;
    }

    Some(JdtImportProgress {
        activity_signature: format!("{kind}|{}|{message}", percentage.unwrap_or(u64::MAX)),
        phase: progress_phase(message),
        percentage,
        current_project: progress_project(message),
        download: progress_download(message),
    })
}

/// Returns whether a Java notification is replaced by structured import logs.
pub fn is_structured_import_notification(provider_id: &str, method: Option<&str>) -> bool {
    is_java_provider(provider_id)
        && matches!(
            method,
            Some(WORK_DONE_PROGRESS_METHOD | LANGUAGE_STATUS_METHOD)
        )
}

fn progress_phase(message: &str) -> Option<String> {
    let phase = message
        .split_once(" - ")
        .map_or(message, |(phase, _)| phase)
        .trim();
    (!phase.is_empty()).then(|| phase.to_string())
}

fn progress_project(message: &str) -> Option<String> {
    if let Some((_, project)) = message.split_once("Importing project ") {
        let project = project.trim().trim_matches('\'');
        return (!project.is_empty()).then(|| project.to_string());
    }
    let (_, quoted) = message.split_once("Project '")?;
    let (project, _) = quoted.split_once('\'')?;
    let project = project.trim();
    (!project.is_empty()).then(|| project.to_string())
}

fn progress_download(message: &str) -> Option<JdtDownloadProgress> {
    let url_text = message
        .split_whitespace()
        .find(|part| part.starts_with("https://") || part.starts_with("http://"))?
        .trim_end_matches([',', ';', ')', ']']);
    let url = Url::parse(url_text).ok()?;
    let artifact = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|segment| !segment.is_empty())?
        .to_string();
    let sizes = message.split_whitespace().find_map(parse_size_pair);
    Some(JdtDownloadProgress {
        artifact,
        repository_host: url.host_str().map(ToString::to_string),
        downloaded_bytes: sizes.map(|(downloaded, _)| downloaded),
        total_bytes: sizes.map(|(_, total)| total),
    })
}

fn parse_size_pair(value: &str) -> Option<(u64, u64)> {
    if value.starts_with("http://") || value.starts_with("https://") {
        return None;
    }
    let value = value.trim_matches(|character: char| matches!(character, '(' | ')' | ',' | ';'));
    let (downloaded, total) = value.split_once('/')?;
    Some((parse_byte_count(downloaded)?, parse_byte_count(total)?))
}

fn parse_byte_count(value: &str) -> Option<u64> {
    let number_end = value
        .find(|character: char| !character.is_ascii_digit() && character != '.')
        .unwrap_or(value.len());
    let number = value[..number_end].parse::<f64>().ok()?;
    let multiplier = match value[number_end..].to_ascii_uppercase().as_str() {
        "B" => 1_f64,
        "KB" | "KIB" => 1024_f64,
        "MB" | "MIB" => 1024_f64 * 1024_f64,
        "GB" | "GIB" => 1024_f64 * 1024_f64 * 1024_f64,
        _ => return None,
    };
    Some((number * multiplier).round() as u64)
}

/// Marks JDT virtual locations as read-only and gives them a source-like path.
/// File locations and locations from other providers pass through unchanged.
pub fn normalize_location(provider_id: &str, mut location: ProviderLocation) -> ProviderLocation {
    if is_java_provider(provider_id) && has_uri_scheme(&location.uri, JDT_URI_SCHEME) {
        location.is_read_only = true;
        location.display_path = jdt_display_path(&location.uri);
    }
    location
}

/// Converts a JDT virtual URI into `workspace/executeCommand` parameters. The
/// generic engine owns request IDs and JSON-RPC framing.
pub fn virtual_source_resolve_params(provider_id: &str, uri: &str) -> Option<ExecuteCommandParams> {
    if !is_virtual_source_uri(provider_id, uri) {
        return None;
    }
    Some(ExecuteCommandParams {
        command: JAVA_DECOMPILE_COMMAND.to_string(),
        arguments: vec![json!(uri)],
    })
}

/// Returns whether a URI names a virtual source document owned by JDT LS.
pub fn is_virtual_source_uri(provider_id: &str, uri: &str) -> bool {
    is_java_provider(provider_id) && has_uri_scheme(uri, JDT_URI_SCHEME)
}

/// Extracts the text shape returned by JDT LS for `java.decompile`.
/// Different JDT LS versions return either the string directly or wrap it in
/// a `content` member.
pub fn virtual_source_content(provider_id: &str, result: &Value) -> Option<String> {
    if !is_java_provider(provider_id) {
        return None;
    }
    result
        .as_str()
        .or_else(|| result.get("content").and_then(Value::as_str))
        .filter(|content| !content.is_empty())
        .map(str::to_string)
}

fn is_java_provider(provider_id: &str) -> bool {
    provider_id.trim().eq_ignore_ascii_case(JAVA_PROVIDER_ID)
}

/// Maximum heap used when the plugin does not choose one, or chooses one the
/// host cannot accept. See `JDTLS_MAX_HEAP_ARGUMENT` for why it is 2 GB.
const DEFAULT_MAX_HEAP_ARGUMENT: &str = JDTLS_MAX_HEAP_ARGUMENT;
/// JDT LS runs its constrained-memory import at or below this heap, so no
/// plugin choice at or below it is accepted.
const CONSTRAINED_HEAP_MEBIBYTES: u64 = 1536;
const MAX_HEAP_PREFIX: &str = "-Xmx";
/// Language-server log level used when the plugin does not choose one.
const DEFAULT_LOG_LEVEL_ARGUMENT: &str = "-Dlog.level=WARNING";
const LOG_LEVEL_PREFIX: &str = "-Dlog.level=";
/// JVM defaults vscode-java and the JDT LS CI launch the language server with.
///
/// The throughput collector with a 20% GC-time budget and a slowly adapting
/// size policy returns heap to the OS once an import settles, instead of
/// holding the 2 GB ceiling for the life of the session. `-Xlog:disable` keeps
/// JVM warnings off stdout, which carries the LSP stream. The installed-VM
/// scan is redundant because the host sends `java.configuration.runtimes`.
/// A plugin option with the same key replaces the default; see
/// `tuning_arguments`.
const JDTLS_TUNING_ARGUMENTS: [&str; 5] = [
    "-XX:+UseParallelGC",
    "-XX:GCTimeRatio=4",
    "-XX:AdaptiveSizePolicyWeight=90",
    "-Xlog:disable",
    "-DDetectVMInstallationsJob.disabled=true",
];

/// Removes every argument starting with `prefix` and returns the last one
/// that is acceptable. The JVM honors the last occurrence, so that is the one
/// the plugin meant; an unacceptable value is dropped so the host default
/// stays in force.
fn take_plugin_choice(
    arguments: &mut Vec<String>,
    prefix: &str,
    acceptable: fn(&str) -> bool,
) -> Option<String> {
    let choice = arguments
        .iter()
        .rev()
        .find(|argument| argument.strip_prefix(prefix).is_some_and(acceptable))
        .cloned();
    arguments.retain(|argument| !argument.starts_with(prefix));
    choice
}

/// A JVM heap size (digits and an optional `k`, `m`, or `g` unit) above the
/// size at which JDT LS switches to its constrained-memory import. A smaller
/// choice is dropped so the host default stays in force.
fn is_heap_size(value: &str) -> bool {
    let digits = value.trim_end_matches(['k', 'K', 'm', 'M', 'g', 'G']);
    if digits.is_empty()
        || value.len() - digits.len() > 1
        || !digits.chars().all(|character| character.is_ascii_digit())
    {
        return false;
    }
    let Ok(amount) = digits.parse::<u64>() else {
        return false;
    };
    let mebibytes = match value[digits.len()..].to_ascii_lowercase().as_str() {
        "k" => amount / 1024,
        "m" => amount,
        "g" => amount.saturating_mul(1024),
        _ => amount / (1024 * 1024),
    };
    mebibytes > CONSTRAINED_HEAP_MEBIBYTES
}

/// Returns the tuning defaults the plugin's JVM options leave unset. An option
/// with the same key replaces a default, and any garbage-collector selection
/// replaces the default collector, since the JVM refuses two collectors.
fn tuning_arguments(custom_jvm_arguments: &[String]) -> Vec<String> {
    let chooses_collector = custom_jvm_arguments
        .iter()
        .any(|argument| is_collector_selection(argument));
    JDTLS_TUNING_ARGUMENTS
        .iter()
        .filter(|default| !(chooses_collector && is_collector_selection(default)))
        .filter(|default| {
            let key = jvm_option_key(default);
            !custom_jvm_arguments
                .iter()
                .any(|argument| jvm_option_key(argument) == key)
        })
        .map(|default| default.to_string())
        .collect()
}

/// A tuning default exactly as the host writes it. It is host-owned, so a
/// relaunch rebuilds it instead of reading it back as a plugin choice.
fn is_tuning_default(argument: &str) -> bool {
    JDTLS_TUNING_ARGUMENTS.contains(&argument)
}

fn is_collector_selection(argument: &str) -> bool {
    argument
        .strip_prefix("-XX:")
        .and_then(|option| option.strip_prefix(['+', '-']))
        .is_some_and(|option| option.starts_with("Use") && option.ends_with("GC"))
}

/// The part of a JVM option that identifies it regardless of its value:
/// `-XX:` options by name without the `+`/`-` toggle, every `-Xlog` option as
/// one key because they combine into a single logging configuration, and other
/// options by the text before `=`.
fn jvm_option_key(argument: &str) -> String {
    if let Some(option) = argument.strip_prefix("-XX:") {
        let option = option.trim_start_matches(['+', '-']);
        let name = option.split_once('=').map_or(option, |(name, _)| name);
        return format!("-XX:{name}");
    }
    if argument.starts_with("-Xlog") {
        return "-Xlog".to_string();
    }
    argument
        .split_once('=')
        .map_or(argument, |(name, _)| name)
        .to_string()
}

fn is_log_level(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
}

fn wrapper_arguments(
    arguments: &[String],
    java_executable: Option<&Path>,
    data_directory: &Path,
) -> Vec<String> {
    let mut jvm_arguments: Vec<String> = arguments
        .iter()
        .filter_map(|argument| argument.strip_prefix("--jvm-arg="))
        .map(str::to_string)
        .collect();
    let max_heap = take_plugin_choice(&mut jvm_arguments, MAX_HEAP_PREFIX, is_heap_size)
        .unwrap_or_else(|| DEFAULT_MAX_HEAP_ARGUMENT.to_string());
    jvm_arguments.retain(|argument| !is_tuning_default(argument));
    let tuning = tuning_arguments(&jvm_arguments);
    let mut arguments = without_wrapper_owned_arguments(arguments);
    if let Some(java_executable) = java_executable {
        arguments.push("--java-executable".to_string());
        arguments.push(java_executable.to_string_lossy().into_owned());
    }
    arguments.extend([
        "--jvm-arg=-Xms256m".to_string(),
        format!("--jvm-arg={max_heap}"),
    ]);
    arguments.extend(
        tuning
            .into_iter()
            .map(|argument| format!("--jvm-arg={argument}")),
    );
    arguments.extend([
        format!("--jvm-arg={METADATA_AT_PROJECT_ROOT_PROPERTY}=false"),
        "-data".to_string(),
        data_directory.to_string_lossy().into_owned(),
    ]);
    arguments
}

fn direct_java_arguments(
    arguments: &[String],
    resources: &JdtDirectLaunchResources,
    data_directory: &Path,
) -> Vec<String> {
    let (mut custom_jvm_arguments, server_arguments) =
        split_direct_launch_arguments(arguments, &resources.launcher_jar_path);
    let max_heap = take_plugin_choice(&mut custom_jvm_arguments, MAX_HEAP_PREFIX, is_heap_size)
        .unwrap_or_else(|| DEFAULT_MAX_HEAP_ARGUMENT.to_string());
    let log_level = take_plugin_choice(&mut custom_jvm_arguments, LOG_LEVEL_PREFIX, is_log_level)
        .unwrap_or_else(|| DEFAULT_LOG_LEVEL_ARGUMENT.to_string());
    let mut adapted = vec![
        format!(
            "-javaagent:{}",
            resources.lombok_agent_path.to_string_lossy()
        ),
        "-Xms256m".to_string(),
        max_heap,
    ];
    adapted.extend(tuning_arguments(&custom_jvm_arguments));
    adapted.extend([
        "--add-modules=ALL-SYSTEM".to_string(),
        "--add-opens=java.base/java.util=ALL-UNNAMED".to_string(),
        "--add-opens=java.base/java.lang=ALL-UNNAMED".to_string(),
        "-Declipse.application=org.eclipse.jdt.ls.core.id1".to_string(),
        "-Declipse.product=org.eclipse.jdt.ls.core.product".to_string(),
        "-Dosgi.bundles.defaultStartLevel=4".to_string(),
        "-Dlog.protocol=true".to_string(),
        log_level,
        format!("{METADATA_AT_PROJECT_ROOT_PROPERTY}=false"),
    ]);
    adapted.extend(custom_jvm_arguments);
    adapted.extend([
        "-jar".to_string(),
        resources.launcher_jar_path.to_string_lossy().into_owned(),
        "-configuration".to_string(),
        resources
            .configuration_directory
            .to_string_lossy()
            .into_owned(),
    ]);
    adapted.extend(server_arguments);
    adapted.extend([
        "-data".to_string(),
        data_directory.to_string_lossy().into_owned(),
    ]);
    adapted
}

fn split_direct_launch_arguments(
    arguments: &[String],
    launcher_jar_path: &Path,
) -> (Vec<String>, Vec<String>) {
    let launcher_jar_path = launcher_jar_path.to_string_lossy();
    let owned_launcher_index = arguments
        .windows(2)
        .position(|pair| pair[0] == "-jar" && pair[1] == launcher_jar_path.as_ref());
    let has_direct_layout = owned_launcher_index.is_some();
    let mut custom_jvm_arguments = Vec::new();
    let mut server_arguments = Vec::new();
    let mut after_launcher = false;
    let mut index = 0;
    while index < arguments.len() {
        let argument = &arguments[index];
        if argument == "--java-executable" || argument == "-data" {
            index += usize::from(index + 1 < arguments.len()) + 1;
            continue;
        }
        if argument == "--jvm-arg" {
            if let Some(value) = arguments.get(index + 1) {
                if !is_jdt_owned_jvm_argument(value) {
                    custom_jvm_arguments.push(value.clone());
                }
            }
            index += usize::from(index + 1 < arguments.len()) + 1;
            continue;
        }
        if let Some(value) = argument.strip_prefix("--jvm-arg=") {
            if !is_jdt_owned_jvm_argument(value) {
                custom_jvm_arguments.push(value.to_string());
            }
            index += 1;
            continue;
        }
        if owned_launcher_index == Some(index) {
            after_launcher = true;
            index += usize::from(index + 1 < arguments.len()) + 1;
            continue;
        }
        if argument == "-configuration" && has_direct_layout && after_launcher {
            index += usize::from(index + 1 < arguments.len()) + 1;
            continue;
        }
        if has_direct_layout
            && !after_launcher
            && ((argument == "--add-modules"
                && arguments
                    .get(index + 1)
                    .is_some_and(|value| value == "ALL-SYSTEM"))
                || (argument == "--add-opens"
                    && arguments.get(index + 1).is_some_and(|value| {
                        value == "java.base/java.util=ALL-UNNAMED"
                            || value == "java.base/java.lang=ALL-UNNAMED"
                    })))
        {
            index += 2;
            continue;
        }
        if argument.starts_with("--java-executable=") || argument.starts_with("-data=") {
            index += 1;
            continue;
        }
        if has_direct_layout && !after_launcher {
            if !is_jdt_owned_jvm_argument(argument) {
                custom_jvm_arguments.push(argument.clone());
            }
        } else if !is_jdt_owned_server_argument(argument) {
            server_arguments.push(argument.clone());
        }
        index += 1;
    }
    (custom_jvm_arguments, server_arguments)
}

fn is_jdt_owned_jvm_argument(argument: &str) -> bool {
    // `-Xmx` and `-Dlog.level=` are the plugin's to choose, so they stay in
    // the custom arguments, where `direct_java_arguments` reads them.
    argument.starts_with("-Xms")
        || argument == "--add-modules=ALL-SYSTEM"
        || argument == "--add-opens=java.base/java.util=ALL-UNNAMED"
        || argument == "--add-opens=java.base/java.lang=ALL-UNNAMED"
        || argument.starts_with("-Declipse.application=")
        || argument.starts_with("-Declipse.product=")
        || argument.starts_with("-Dosgi.bundles.defaultStartLevel=")
        || argument.starts_with("-Dlog.protocol=")
        || is_tuning_default(argument)
        || is_metadata_location_argument(argument)
        || is_lombok_agent_argument(argument)
}

fn is_metadata_location_argument(argument: &str) -> bool {
    argument
        .strip_prefix(METADATA_AT_PROJECT_ROOT_PROPERTY)
        .is_some_and(|rest| rest.starts_with('='))
}

fn is_lombok_agent_argument(argument: &str) -> bool {
    let Some(path) = argument.strip_prefix("-javaagent:") else {
        return false;
    };
    path.split_once('=')
        .map_or(path, |(path, _)| path)
        .replace('\\', "/")
        .rsplit('/')
        .next()
        .is_some_and(|name| name.eq_ignore_ascii_case("lombok.jar"))
}

fn is_jdt_owned_server_argument(argument: &str) -> bool {
    argument == "-data" || argument.starts_with("-data=")
}

fn without_wrapper_owned_arguments(arguments: &[String]) -> Vec<String> {
    let mut retained = Vec::with_capacity(arguments.len());
    let mut index = 0;
    while index < arguments.len() {
        let argument = &arguments[index];
        let owns_following_value = argument == "--java-executable" || argument == "-data";
        let owns_inline_value = argument.starts_with("--java-executable=")
            || argument.starts_with("-data=")
            || argument.starts_with("--jvm-arg=-Xms")
            || argument.starts_with("--jvm-arg=-Xmx")
            || argument.strip_prefix("--jvm-arg=").is_some_and(|value| {
                is_metadata_location_argument(value) || is_tuning_default(value)
            });
        if owns_following_value {
            index += usize::from(index + 1 < arguments.len()) + 1;
        } else {
            if !owns_inline_value {
                retained.push(argument.clone());
            }
            index += 1;
        }
    }
    retained
}

fn java_settings(settings: JdtSettings<'_>) -> Value {
    let mut java = json!({
        "java": {
            "eclipse": {
                "downloadSources": false
            },
            "maven": {
                "downloadSources": false
            },
            "configuration": {
                "updateBuildConfiguration": "automatic"
            },
            // JDT LS otherwise inserts parameter names into calls. Its "off"
            // mode keeps the argument list empty and the snippet cursor inside.
            "completion": java_completion_settings(settings.completion),
            "inlayHints": {
                "parameterNames": {
                    "enabled": "all"
                }
            },
            // Show "N implementations" code lens above interface methods and classes.
            "implementationsCodeLens": {
                "enabled": true
            },
            // Show "N references" code lens above declarations.
            "referencesCodeLens": {
                "enabled": true
            }
        }
    });
    // Contributed keys never overlap the keys set above or below: none of
    // them lies inside `CONTRIBUTED_SETTINGS_KEYS`.
    for fragment in settings.contributed {
        for (key, value) in accept_contributed_settings(fragment).accepted {
            insert_dotted(&mut java, &key, value);
        }
    }
    if settings.indexing_root_uri.is_some() || settings.import_exclusions.is_some() {
        let mut exclusions = settings
            .import_exclusions
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if let Some(root_uri) = settings.indexing_root_uri {
            for path in crate::host::indexing::java_import_exclusions(root_uri) {
                let value = Value::String(path);
                if !exclusions.contains(&value) {
                    exclusions.push(value);
                }
            }
        }
        java["java"]["import"]["exclusions"] = json!(exclusions);
    }
    if !settings.java_runtimes.is_empty() {
        java["java"]["configuration"]["runtimes"] = java_runtimes_value(settings.java_runtimes);
    }
    java
}

/// Stores `value` at a dotted key such as `java.import.gradle.offline.enabled`,
/// creating objects along the way and replacing non-object values in the path.
fn insert_dotted(root: &mut Value, key: &str, value: Value) {
    let mut target = root;
    for segment in key.split('.') {
        if !target.is_object() {
            *target = json!({});
        }
        target = target
            .as_object_mut()
            .expect("the path value was normalized to an object")
            .entry(segment.to_string())
            .or_insert(Value::Null);
    }
    *target = value;
}

/// Retains the plugin's completion leaves while applying Lithe's argument policy.
/// Invalid section shapes are normalized without mutating the session input.
fn java_completion_settings(declared: Option<&Value>) -> Value {
    let mut completion = declared
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    completion.insert("guessMethodArguments".to_string(), json!("off"));
    Value::Object(completion)
}

fn java_runtimes_value(runtimes: &[JdtJavaRuntime]) -> Value {
    Value::Array(
        runtimes
            .iter()
            .map(|runtime| {
                let mut value = json!({ "name": runtime.name, "path": runtime.path });
                if runtime.is_default {
                    value["default"] = json!(true);
                }
                value
            })
            .collect(),
    )
}

/// Answers a `workspace/configuration` item from the same settings sent at
/// startup, so a section request and the initialization options never
/// disagree. Unknown sections answer `null`, which JDT LS treats as "use the
/// default".
fn java_configuration_for_section(section: Option<&str>, settings: JdtSettings<'_>) -> Value {
    let Some(section) = section else {
        return Value::Null;
    };
    if section == "java.configuration.runtimes" {
        // Answered with a list even when no runtime is registered; the
        // initialization options omit an empty list instead.
        return java_runtimes_value(settings.java_runtimes);
    }
    let settings = java_settings(settings);
    section
        .split('.')
        .try_fold(&settings, |value, key| value.get(key))
        .cloned()
        .unwrap_or(Value::Null)
}

/// Resolves a platform request through the same key algorithm used at startup.
pub fn resolve_workspace_key(request: JdtWorkspaceKeyRequest) -> JdtWorkspaceKeyResponse {
    JdtWorkspaceKeyResponse {
        workspace_key: workspace_key(
            Path::new(&request.workspace_root),
            request.workspace_fingerprint.as_deref(),
        ),
    }
}

/// Hashes the normalized workspace identity and optional structural fingerprint.
pub fn workspace_key(workspace_root: &Path, fingerprint: Option<&str>) -> String {
    let mut identity = normalized_workspace_identity(workspace_root);
    // Mix the structural fingerprint in as a null-delimited suffix so changes
    // to build files or the module set produce a fresh state directory without
    // touching the previous one.  Absent fingerprint keeps the legacy key.
    if let Some(fp) = fingerprint {
        identity.push('\x00');
        identity.push_str(fp);
    }
    let digest = Sha256::digest(identity.as_bytes());
    let mut key = String::with_capacity(digest.len() * 2);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in digest {
        key.push(HEX[(byte >> 4) as usize] as char);
        key.push(HEX[(byte & 0x0f) as usize] as char);
    }
    key
}

fn normalized_workspace_identity(workspace_root: &Path) -> String {
    let raw = workspace_root.to_string_lossy().replace('\\', "/");
    let bytes = raw.as_bytes();
    let has_drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    let is_unc = raw.starts_with("//");
    let (prefix, remainder, is_absolute, protected_components, fold_case) = if has_drive {
        let drive = (bytes[0] as char).to_ascii_lowercase();
        let remainder = &raw[2..];
        (
            format!("{drive}:"),
            remainder,
            remainder.starts_with('/'),
            0,
            true,
        )
    } else if is_unc {
        ("//".to_string(), raw.trim_start_matches('/'), true, 2, true)
    } else if raw.starts_with('/') {
        ("/".to_string(), raw.trim_start_matches('/'), true, 0, false)
    } else {
        (String::new(), raw.as_str(), false, 0, false)
    };

    let mut components: Vec<String> = Vec::new();
    for component in remainder.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                if components.len() > protected_components
                    && components.last().is_some_and(|value| value != "..")
                {
                    components.pop();
                } else if !is_absolute {
                    components.push("..".to_string());
                }
            }
            value => components.push(if fold_case {
                value.to_lowercase()
            } else {
                value.to_string()
            }),
        }
    }

    let joined = components.join("/");
    match (prefix.as_str(), joined.is_empty(), is_absolute) {
        ("", true, _) => ".".to_string(),
        ("/", true, _) => "/".to_string(),
        ("//", true, _) => "//".to_string(),
        (prefix, true, _) => prefix.to_string(),
        ("", false, _) => joined,
        ("/", false, _) => format!("/{joined}"),
        ("//", false, _) => format!("//{joined}"),
        (prefix, false, true) => format!("{prefix}/{joined}"),
        (prefix, false, false) => format!("{prefix}{joined}"),
    }
}

fn has_uri_scheme(uri: &str, expected: &str) -> bool {
    uri.split_once(':')
        .is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case(expected))
}

fn jdt_display_path(uri: &str) -> Option<String> {
    let (_, remainder) = uri.split_once(':')?;
    let path = if let Some(with_authority) = remainder.strip_prefix("//") {
        with_authority
            .find('/')
            .map(|index| &with_authority[index + 1..])?
    } else {
        remainder.trim_start_matches('/')
    };
    let path = path.split_once(['?', '#']).map_or(path, |(value, _)| value);
    let decoded = percent_decode(path);
    let mut components: Vec<_> = decoded
        .split('/')
        .filter(|component| !component.is_empty())
        .map(str::to_string)
        .collect();
    let last = components.last_mut()?;
    if let Some(class_name) = last.strip_suffix(".class") {
        *last = format!("{class_name}.java");
    }
    Some(components.join("/"))
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
            {
                decoded.push((high << 4) | low);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime(home_path: &str, version: &str) -> JavaRuntimeCandidate {
        JavaRuntimeCandidate {
            is_default: false,
            home_path: home_path.to_string(),
            version: version.to_string(),
        }
    }

    #[test]
    fn one_jdk_per_major_version_becomes_a_jdt_execution_environment() {
        // A Java 25 project only builds when JDT LS knows a JavaSE-25 JDK; the
        // bundled JDK 21 it runs on is not enough (#769).
        let runtimes = jdt_java_runtimes(&[
            runtime("/jdks/temurin-25", "25.0.4.1"),
            runtime("/jdks/corretto-25", "25.0.1"),
            runtime("/jdks/jdk8", "1.8.0_402"),
            runtime("/jdks/unknown", "not-a-version"),
            runtime("", "17.0.9"),
            runtime("/jdks/temurin-21", "21"),
        ]);
        assert_eq!(
            runtimes,
            vec![
                JdtJavaRuntime {
                    is_default: false,
                    name: "JavaSE-1.8".to_string(),
                    path: "/jdks/jdk8".to_string()
                },
                JdtJavaRuntime {
                    is_default: false,
                    name: "JavaSE-21".to_string(),
                    path: "/jdks/temurin-21".to_string()
                },
                JdtJavaRuntime {
                    is_default: false,
                    name: "JavaSE-25".to_string(),
                    path: "/jdks/temurin-25".to_string()
                },
            ]
        );
    }

    #[test]
    fn selected_project_vm_wins_over_discovery_order_and_duplicate_majors() {
        let fixture: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/lsp/java-project-runtimes-v1.json"
        )))
        .unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let candidates: Vec<JavaRuntimeCandidate> =
                serde_json::from_value(case["candidates"].clone()).unwrap();
            assert_eq!(
                java_runtimes_value(&jdt_java_runtimes(&candidates)),
                case["expected"],
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn java_runtimes_reach_jdt_at_initialize_and_on_configuration_requests() {
        let runtimes = [JdtJavaRuntime {
            is_default: true,
            name: "JavaSE-25".to_string(),
            path: "C:/Program Files/Java/jdk-25".to_string(),
        }];
        let settings = JdtSettings {
            java_runtimes: &runtimes,
            ..Default::default()
        };
        let expected = json!([{ "name": "JavaSE-25", "path": "C:/Program Files/Java/jdk-25", "default": true }]);
        let options = adapt_initialization_options("java", None, &[], settings).unwrap();
        assert_eq!(
            options["settings"]["java"]["configuration"]["runtimes"],
            expected
        );
        let items = ["java", "java.configuration", "java.configuration.runtimes"].map(|section| {
            WorkspaceConfigurationItem {
                scope_uri: None,
                section: Some(section.to_string()),
            }
        });
        let values = workspace_configuration("java", &items, settings).unwrap();
        assert_eq!(values[0]["configuration"]["runtimes"], expected);
        assert_eq!(values[1]["runtimes"], expected);
        assert_eq!(values[2], expected);
        let notification = initialized_notification("java", settings).unwrap();
        assert_eq!(
            notification.params["settings"]["java"]["configuration"]["runtimes"],
            expected
        );
    }

    #[test]
    fn no_known_jdks_leaves_jdt_on_its_own_runtime() {
        let options =
            adapt_initialization_options("java", None, &[], JdtSettings::default()).unwrap();
        assert!(options["settings"]["java"]["configuration"]
            .get("runtimes")
            .is_none());
    }

    fn contributed_settings(fragments: &[Value]) -> JdtSettings<'_> {
        JdtSettings {
            contributed: fragments,
            ..Default::default()
        }
    }

    #[test]
    fn contributed_fragments_keep_only_the_declared_jdtls_keys() {
        // Nested objects, dotted keys and a mix of both reach the same leaves.
        let fragment = json!({
            "java": {
                "configuration": {
                    "maven": { "globalSettings": "/state/maven/settings.xml" },
                    "runtimes": []
                },
                "import.gradle": { "offline": { "enabled": true } },
                "project": { "sourcePaths": ["src/main/java"], "outputPath": "bin" },
                "maven": { "downloadSources": true }
            },
            "java.import.maven.enabled": true,
            "java.import.gradle": false,
            "java.import.gradle..home": "/empty-segment",
            "java.project.sourcePaths.extra": 1,
            "editor": { "tabSize": 2 }
        });

        let contributed = accept_contributed_settings(&fragment);

        assert_eq!(
            contributed.accepted,
            BTreeMap::from([
                (
                    "java.configuration.maven.globalSettings".to_string(),
                    json!("/state/maven/settings.xml")
                ),
                (
                    "java.import.gradle.offline.enabled".to_string(),
                    json!(true)
                ),
                ("java.import.maven.enabled".to_string(), json!(true)),
                (
                    "java.project.sourcePaths".to_string(),
                    json!(["src/main/java"])
                ),
            ])
        );
        let mut rejected = contributed.rejected;
        rejected.sort();
        // A prefix entry accepts keys below it, never the namespace itself;
        // an exact entry accepts nothing below it.
        assert_eq!(
            rejected,
            [
                "editor.tabSize",
                "java.configuration.runtimes",
                "java.import.gradle",
                "java.import.gradle..home",
                "java.maven.downloadSources",
                "java.project.outputPath",
                "java.project.sourcePaths.extra",
            ]
        );
        for fragment in [json!(null), json!("java.import.maven.enabled"), json!([{}])] {
            assert_eq!(
                accept_contributed_settings(&fragment),
                ContributedSettings::default()
            );
        }
    }

    #[test]
    fn contributed_settings_reach_jdt_before_the_first_import() {
        // JDT LS configures its Maven embedder and Buildship while handling
        // `initialize`, and the project import starts right after. Settings
        // sent only through `didChangeConfiguration` would arrive once that
        // import had already resolved every artifact against the defaults.
        let fragments = [
            json!({
                "java.configuration.maven.globalSettings": "/state/maven/settings.xml",
                "java.project.sourcePaths": ["src/main/java"],
                // Owned by this plugin, so a contribution cannot change it.
                "java.maven.downloadSources": true
            }),
            json!({
                "java": { "import": { "gradle": { "enabled": true, "wrapper": { "enabled": true } } } }
            }),
        ];

        let options = adapt_initialization_options(
            "java",
            Some(json!({
                "workspace": { "custom": true },
                "settings": { "java": { "custom": true } }
            })),
            &[],
            contributed_settings(&fragments),
        )
        .unwrap();

        let java = &options["settings"]["java"];
        assert_eq!(
            java["configuration"]["maven"]["globalSettings"],
            "/state/maven/settings.xml"
        );
        assert_eq!(
            java["configuration"]["updateBuildConfiguration"],
            "automatic"
        );
        assert_eq!(java["project"]["sourcePaths"], json!(["src/main/java"]));
        assert_eq!(java["import"]["gradle"]["enabled"], true);
        assert_eq!(java["import"]["gradle"]["wrapper"]["enabled"], true);
        assert_eq!(java["maven"]["downloadSources"], false);
        // Catalog values outside the provider-owned keys survive the overlay.
        assert_eq!(java["custom"], true);
        assert_eq!(options["workspace"]["custom"], true);
    }

    #[test]
    fn later_contributions_replace_earlier_ones_leaf_by_leaf() {
        let fragments = [
            json!({
                "java.import.gradle.offline.enabled": true,
                "java.import.gradle.user.home": "/caches/first"
            }),
            json!({ "java": { "import": { "gradle": { "user": { "home": "/caches/second" } } } } }),
        ];

        let options =
            adapt_initialization_options("java", None, &[], contributed_settings(&fragments))
                .unwrap();

        let gradle = &options["settings"]["java"]["import"]["gradle"];
        assert_eq!(gradle["offline"]["enabled"], true);
        assert_eq!(gradle["user"]["home"], "/caches/second");
    }

    #[test]
    fn contributed_import_settings_keep_the_catalog_import_options() {
        // A contribution states only the leaves its plugin knows; a declared
        // `import.gradle` subtree has to survive the overlay instead of being
        // replaced by those leaves.
        let fragments = [json!({
            "java.import.gradle.enabled": true,
            "java.import.gradle.wrapper.enabled": true
        })];

        let options = adapt_initialization_options(
            "java",
            Some(json!({
                "settings": {
                    "java": {
                        "import": {
                            "gradle": {
                                "offline": { "enabled": true },
                                "version": "8.10"
                            }
                        }
                    }
                }
            })),
            &[],
            contributed_settings(&fragments),
        )
        .unwrap();

        let gradle = &options["settings"]["java"]["import"]["gradle"];
        assert_eq!(gradle["enabled"], true);
        assert_eq!(gradle["wrapper"]["enabled"], true);
        assert_eq!(gradle["offline"]["enabled"], true);
        assert_eq!(gradle["version"], "8.10");
    }

    #[test]
    fn workspace_configuration_and_notifications_answer_contributed_sections() {
        let fragments = [
            json!({
                "java.configuration.maven.userSettings": "/local/settings.xml",
                "java.configuration.maven.globalSettings": "/state/maven/settings.xml",
                "java.project.sourcePaths": ["src/main/java", "module-a/src/main/java"]
            }),
            json!({ "java.import.gradle.enabled": true, "java.import.gradle.wrapper.enabled": true }),
        ];
        let settings = contributed_settings(&fragments);
        let items = [
            "java.configuration.maven",
            "java.configuration.maven.userSettings",
            "java.configuration.maven.globalSettings",
            "java.import",
            "java.import.gradle.wrapper",
            "java.import.gradle.wrapper.enabled",
            "java.import.gradle.version",
            "java.project",
            "java.project.sourcePaths",
        ]
        .map(|section| WorkspaceConfigurationItem {
            scope_uri: Some("file:///workspace/reactor/".to_string()),
            section: Some(section.to_string()),
        });

        let values = workspace_configuration("java", &items, settings).unwrap();
        assert_eq!(
            values[0],
            json!({
                "userSettings": "/local/settings.xml",
                "globalSettings": "/state/maven/settings.xml"
            })
        );
        assert_eq!(values[1], "/local/settings.xml");
        assert_eq!(values[2], "/state/maven/settings.xml");
        assert_eq!(values[3]["gradle"]["enabled"], true);
        assert_eq!(values[4], json!({ "enabled": true }));
        assert_eq!(values[5], true);
        // A key no plugin contributed stays on JDT LS's own default.
        assert_eq!(values[6], Value::Null);
        let source_paths = json!(["src/main/java", "module-a/src/main/java"]);
        assert_eq!(values[7], json!({ "sourcePaths": source_paths }));
        assert_eq!(values[8], source_paths);

        for notification in [
            initialized_notification("java", settings).unwrap(),
            settings_notification(settings),
        ] {
            let java = &notification.params["settings"]["java"];
            assert_eq!(
                java["configuration"]["maven"]["userSettings"],
                "/local/settings.xml"
            );
            assert_eq!(java["import"]["gradle"]["wrapper"]["enabled"], true);
            assert_eq!(java["project"]["sourcePaths"], source_paths);
        }
    }

    #[test]
    fn project_updates_name_each_project_once_in_first_seen_order() {
        let uris = [
            "file:///workspace/reactor/",
            "file:///workspace/reactor/module-a/",
            "file:///workspace/reactor/",
            "file:///workspace/reactor/module-a/nested/",
        ]
        .map(str::to_string);

        let notification = project_update_notification(&uris).unwrap();

        assert_eq!(notification.method, "java/projectConfigurationsUpdate");
        assert_eq!(
            notification.params,
            json!({
                "identifiers": [
                    { "uri": "file:///workspace/reactor/" },
                    { "uri": "file:///workspace/reactor/module-a/" },
                    { "uri": "file:///workspace/reactor/module-a/nested/" }
                ]
            })
        );
        assert!(project_update_notification(&[]).is_none());
    }

    fn java_start_context() -> JdtStartContext {
        JdtStartContext {
            provider_id: "java".to_string(),
            workspace_root: PathBuf::from("/workspace/project"),
            data_root: PathBuf::from("/cache/Lithe"),
            selected_java_executable: Some(PathBuf::from("/jdk/bin/java")),
            direct_launch_resources: None,
            arguments: vec!["--stdio".to_string()],
            workspace_fingerprint: None,
        }
    }

    #[test]
    fn import_progress_extracts_module_and_download_diagnostics() {
        let module = import_progress(
            "java",
            Some("$/progress"),
            Some(&json!({
                "value": {
                    "kind": "report",
                    "message": "Importing Maven project(s) - Importing project module-a",
                    "percentage": 24
                }
            })),
        )
        .expect("Java work-done progress should be observed");
        assert_eq!(module.phase.as_deref(), Some("Importing Maven project(s)"));
        assert_eq!(module.current_project.as_deref(), Some("module-a"));
        assert_eq!(module.percentage, Some(24));

        let download = import_progress(
            "java",
            Some("$/progress"),
            Some(&json!({
                "value": {
                    "kind": "report",
                    "message": "Importing Maven project(s) - 117KB/7MB (1%) https://repo.maven.apache.org/maven2/org/apache/poi/poi.jar",
                    "percentage": 49
                }
            })),
        )
        .and_then(|progress| progress.download)
        .expect("artifact transfer details should be parsed");
        assert_eq!(download.artifact, "poi.jar");
        assert_eq!(
            download.repository_host.as_deref(),
            Some("repo.maven.apache.org")
        );
        assert_eq!(download.downloaded_bytes, Some(117 * 1024));
        assert_eq!(download.total_bytes, Some(7 * 1024 * 1024));
    }

    #[test]
    fn import_progress_ignores_other_providers_and_status_text() {
        let params = json!({
            "value": {
                "kind": "report",
                "message": "Importing Maven project(s) - 50%",
                "percentage": 50
            }
        });
        assert!(import_progress("gopls", Some("$/progress"), Some(&params)).is_none());
        assert!(import_progress("java", Some("language/status"), Some(&params)).is_none());
    }

    #[test]
    fn java_start_adds_runtime_memory_and_unique_data_arguments() {
        let context = java_start_context();
        let adapted = adapt_start(&context);
        let data_directory = adapted.data_directory.as_ref().unwrap();

        assert_eq!(
            data_directory.parent().unwrap(),
            Path::new("/cache/Lithe/jdtls")
        );
        assert_eq!(
            data_directory.file_name().unwrap().to_string_lossy().len(),
            64
        );
        assert_eq!(
            adapted.arguments,
            vec![
                "--stdio",
                "--java-executable",
                "/jdk/bin/java",
                "--jvm-arg=-Xms256m",
                "--jvm-arg=-Xmx2048m",
                "--jvm-arg=-XX:+UseParallelGC",
                "--jvm-arg=-XX:GCTimeRatio=4",
                "--jvm-arg=-XX:AdaptiveSizePolicyWeight=90",
                "--jvm-arg=-Xlog:disable",
                "--jvm-arg=-DDetectVMInstallationsJob.disabled=true",
                "--jvm-arg=-Djava.import.generatesMetadataFilesAtProjectRoot=false",
                "-data",
                data_directory.to_string_lossy().as_ref()
            ]
        );
    }

    #[test]
    fn java_start_lets_plugin_jvm_options_replace_the_tuning_defaults() {
        let tuning_of = |arguments: &[String]| {
            arguments
                .iter()
                .filter_map(|argument| argument.strip_prefix("--jvm-arg="))
                .filter(|argument| argument.starts_with("-XX:") || argument.starts_with("-Xlog"))
                .map(str::to_string)
                .collect::<Vec<_>>()
        };
        let mut context = java_start_context();
        context.arguments = vec![
            "--jvm-arg=-XX:+UseG1GC".to_string(),
            "--jvm-arg=-XX:GCTimeRatio=9".to_string(),
            "--jvm-arg=-Xlog:gc*:file=/tmp/gc.log".to_string(),
        ];
        let first = adapt_start(&context);
        context.arguments = first.arguments.clone();
        let second = adapt_start(&context);

        assert_eq!(first, second);
        // The plugin's collector replaces ParallelGC, and its own GC-time
        // ratio and logging replace the host's; the untouched default stays.
        assert_eq!(
            tuning_of(&first.arguments),
            vec![
                "-XX:+UseG1GC",
                "-XX:GCTimeRatio=9",
                "-Xlog:gc*:file=/tmp/gc.log",
                "-XX:AdaptiveSizePolicyWeight=90",
            ]
        );
        assert!(first
            .arguments
            .contains(&"--jvm-arg=-DDetectVMInstallationsJob.disabled=true".to_string()));
    }

    #[test]
    fn java_start_uses_the_plugins_heap_choice_and_is_stable() {
        let mut context = java_start_context();
        context.arguments = vec![
            "--java-executable".to_string(),
            "/old/java".to_string(),
            "--jvm-arg=-Xms2g".to_string(),
            "--jvm-arg=-Xmx4g".to_string(),
            "--jvm-arg=-Duser.language=en".to_string(),
            // A catalog or user override must not put metadata back into the project.
            "--jvm-arg=-Djava.import.generatesMetadataFilesAtProjectRoot=true".to_string(),
            "-data".to_string(),
            "/old/data".to_string(),
        ];
        let first = adapt_start(&context);
        context.arguments = first.arguments.clone();
        let second = adapt_start(&context);

        assert_eq!(first, second);
        assert!(first
            .arguments
            .contains(&"--jvm-arg=-Duser.language=en".to_string()));
        // The plugin chooses the maximum heap; the initial heap stays the host's.
        assert_eq!(
            first
                .arguments
                .iter()
                .filter(|argument| argument.starts_with("--jvm-arg=-Xm"))
                .collect::<Vec<_>>(),
            vec!["--jvm-arg=-Xms256m", "--jvm-arg=-Xmx4g"]
        );
        assert!(!first.arguments.contains(&"/old/java".to_string()));
        assert!(!first.arguments.contains(&"/old/data".to_string()));
        assert_eq!(
            first
                .arguments
                .iter()
                .filter(|argument| argument.contains("generatesMetadataFilesAtProjectRoot"))
                .collect::<Vec<_>>(),
            vec!["--jvm-arg=-Djava.import.generatesMetadataFilesAtProjectRoot=false"]
        );
    }

    #[test]
    fn java_start_keeps_the_heap_above_the_jdtls_constrained_memory_threshold() {
        // JDT LS switches to a batched Maven import whose `updateProjects` path
        // dereferences a `null` project for modules that share a build
        // coordinate (eclipse-jdtls/eclipse.jdt.ls#3893). Staying above 1.5 GB
        // keeps a normal multi-module workspace on the regular import path.
        const CONSTRAINED_MEMORY_THRESHOLD_MEBIBYTES: u64 = 1536;

        let mut direct_context = java_start_context();
        direct_context.direct_launch_resources = Some(JdtDirectLaunchResources {
            launcher_jar_path: PathBuf::from("/jdtls/plugins/equinox.jar"),
            configuration_directory: PathBuf::from("/jdtls/config_mac"),
            lombok_agent_path: PathBuf::from("/jdtls/lombok/lombok.jar"),
            java_debug_bundle_path: None,
        });
        let wrapper = adapt_start(&java_start_context());
        let direct = adapt_start(&direct_context);

        for arguments in [&wrapper.arguments, &direct.arguments] {
            let heap = arguments
                .iter()
                .find_map(|argument| {
                    argument
                        .strip_prefix("--jvm-arg=")
                        .unwrap_or(argument)
                        .strip_prefix("-Xmx")
                })
                .expect("both JDT LS launch paths set a maximum heap");
            let mebibytes = heap
                .strip_suffix('m')
                .and_then(|value| value.parse::<u64>().ok())
                .expect("the JDT LS maximum heap uses a mebibyte suffix");
            assert!(
                mebibytes > CONSTRAINED_MEMORY_THRESHOLD_MEBIBYTES,
                "JDT LS needs more than {CONSTRAINED_MEMORY_THRESHOLD_MEBIBYTES} MiB of maximum heap, found {heap}"
            );
        }
    }

    #[test]
    fn java_direct_start_builds_complete_shell_free_arguments_and_is_stable() {
        let mut context = java_start_context();
        context.direct_launch_resources = Some(JdtDirectLaunchResources {
            launcher_jar_path: PathBuf::from("/jdtls/plugins/equinox.jar"),
            configuration_directory: PathBuf::from("/jdtls/config_mac"),
            lombok_agent_path: PathBuf::from("/jdtls/lombok/lombok.jar"),
            java_debug_bundle_path: Some(PathBuf::from(
                "/jdtls/java-debug/com.microsoft.java.debug.plugin-0.53.1.jar",
            )),
        });
        context.arguments = vec![
            "--stdio".to_string(),
            "-jar".to_string(),
            "/server/extension.jar".to_string(),
            "--java-executable=/old/java".to_string(),
            "--jvm-arg=-Xmx4g".to_string(),
            "--jvm-arg=-Dlog.level=INFO".to_string(),
            "--jvm-arg=-Duser.language=en".to_string(),
            "--jvm-arg=-Dosgi.logfile=/session/plugins/java/files/jdt.log".to_string(),
            "--jvm-arg=-Declipse.log.include.commandline=false".to_string(),
            "--jvm-arg=-Declipse.log.size.max=1024".to_string(),
            "--jvm-arg=-Declipse.log.backup.max=2".to_string(),
            "--jvm-arg=-Declipse.log.level=WARNING".to_string(),
            "--jvm-arg=-Djava.import.generatesMetadataFilesAtProjectRoot=true".to_string(),
            "-data=/old/data".to_string(),
        ];

        let first = adapt_start(&context);
        context.arguments = first.arguments.clone();
        let second = adapt_start(&context);
        let data_directory = first.data_directory.as_ref().unwrap();

        assert_eq!(first, second);
        assert_eq!(first.executable, Some(PathBuf::from("/jdk/bin/java")));
        assert_eq!(
            first.arguments,
            vec![
                "-javaagent:/jdtls/lombok/lombok.jar",
                "-Xms256m",
                "-Xmx4g",
                "-XX:+UseParallelGC",
                "-XX:GCTimeRatio=4",
                "-XX:AdaptiveSizePolicyWeight=90",
                "-Xlog:disable",
                "-DDetectVMInstallationsJob.disabled=true",
                "--add-modules=ALL-SYSTEM",
                "--add-opens=java.base/java.util=ALL-UNNAMED",
                "--add-opens=java.base/java.lang=ALL-UNNAMED",
                "-Declipse.application=org.eclipse.jdt.ls.core.id1",
                "-Declipse.product=org.eclipse.jdt.ls.core.product",
                "-Dosgi.bundles.defaultStartLevel=4",
                "-Dlog.protocol=true",
                "-Dlog.level=INFO",
                "-Djava.import.generatesMetadataFilesAtProjectRoot=false",
                "-Duser.language=en",
                "-Dosgi.logfile=/session/plugins/java/files/jdt.log",
                "-Declipse.log.include.commandline=false",
                "-Declipse.log.size.max=1024",
                "-Declipse.log.backup.max=2",
                "-Declipse.log.level=WARNING",
                "-jar",
                "/jdtls/plugins/equinox.jar",
                "-configuration",
                "/jdtls/config_mac",
                "--stdio",
                "-jar",
                "/server/extension.jar",
                "-data",
                data_directory.to_string_lossy().as_ref(),
            ]
        );
        assert!(!first.arguments.contains(&"/old/java".to_string()));
        assert!(!first.arguments.contains(&"/old/data".to_string()));
        assert!(!first.arguments.contains(&"-Dlog.level=ALL".to_string()));
    }

    #[test]
    fn direct_start_keeps_host_defaults_without_a_choice_and_ignores_malformed_ones() {
        let resources = JdtDirectLaunchResources {
            launcher_jar_path: PathBuf::from("/jdtls/plugins/equinox.jar"),
            configuration_directory: PathBuf::from("/jdtls/config_mac"),
            lombok_agent_path: PathBuf::from("/jdtls/lombok/lombok.jar"),
            java_debug_bundle_path: None,
        };
        let data = Path::new("/data");
        let heap_and_level = |arguments: &[&str]| {
            let arguments: Vec<String> = arguments.iter().map(|value| value.to_string()).collect();
            let adapted = direct_java_arguments(&arguments, &resources, data);
            (
                adapted
                    .iter()
                    .filter(|argument| argument.starts_with("-Xmx"))
                    .cloned()
                    .collect::<Vec<_>>(),
                adapted
                    .iter()
                    .filter(|argument| argument.starts_with("-Dlog.level="))
                    .cloned()
                    .collect::<Vec<_>>(),
            )
        };

        assert_eq!(
            heap_and_level(&[]),
            (
                vec!["-Xmx2048m".to_string()],
                vec!["-Dlog.level=WARNING".to_string()]
            )
        );
        // The last acceptable choice wins; a malformed one never reaches the JVM.
        assert_eq!(
            heap_and_level(&[
                "--jvm-arg=-Xmx2g",
                "--jvm-arg=-Xmx3072m",
                "--jvm-arg=-Dlog.level=WARNING",
                "--jvm-arg=-Dlog.level=SEV ERE",
            ]),
            (
                vec!["-Xmx3072m".to_string()],
                vec!["-Dlog.level=WARNING".to_string()]
            )
        );
        assert_eq!(
            heap_and_level(&[
                "--jvm-arg=-Xmx",
                "--jvm-arg=-Xmxlots",
                "--jvm-arg=-Xmx1024m",
                "--jvm-arg=-Xmx1536m",
                "--jvm-arg=-Dlog.level=",
            ]),
            (
                vec!["-Xmx2048m".to_string()],
                vec!["-Dlog.level=WARNING".to_string()]
            )
        );
        // A plugin that launches the server as `java <jvm options> -jar <launcher>`
        // puts its choices before the launcher, where they are read the same way.
        assert_eq!(
            heap_and_level(&[
                "-Xmx2048m",
                "-Dlog.level=SEVERE",
                "-jar",
                "/jdtls/plugins/equinox.jar",
            ]),
            (
                vec!["-Xmx2048m".to_string()],
                vec!["-Dlog.level=SEVERE".to_string()]
            )
        );
    }

    #[test]
    fn workspace_identity_is_lexical_cross_platform_and_unique() {
        assert_eq!(
            workspace_key(Path::new(r"C:\Users\Ada\Project\.\src\.."), None),
            workspace_key(Path::new("c:/users/ada/project"), None)
        );
        assert_eq!(
            workspace_key(Path::new("/workspace/project/./src/.."), None),
            workspace_key(Path::new("/workspace/project"), None)
        );
        assert_ne!(
            workspace_key(Path::new("/workspace/project"), None),
            workspace_key(Path::new("/workspace/other"), None)
        );
    }

    #[test]
    fn workspace_fingerprint_changes_data_directory() {
        let mut context = java_start_context();
        let without = adapt_start(&context).data_directory.unwrap();
        context.workspace_fingerprint = Some("pom=111|modules=core,web|jdtls=1.38.0".to_string());
        let first = adapt_start(&context).data_directory.unwrap();
        context.workspace_fingerprint = Some("pom=222|modules=core,web|jdtls=1.38.0".to_string());
        let changed = adapt_start(&context).data_directory.unwrap();

        assert_ne!(without, first);
        assert_ne!(first, changed);
        assert_eq!(without.parent(), first.parent());
        assert_eq!(first.parent(), changed.parent());
        assert_eq!(without.file_name().unwrap().to_string_lossy().len(), 64);
        assert_eq!(first.file_name().unwrap().to_string_lossy().len(), 64);
        assert_eq!(changed.file_name().unwrap().to_string_lossy().len(), 64);

        context.workspace_fingerprint = Some("pom=111|modules=core,web|jdtls=1.38.0".to_string());
        let repeated = adapt_start(&context).data_directory.unwrap();
        assert_eq!(first, repeated);
    }

    #[test]
    fn workspace_key_request_uses_the_same_compatibility_algorithm() {
        let response = resolve_workspace_key(JdtWorkspaceKeyRequest {
            workspace_root: "/workspace/project".to_string(),
            workspace_fingerprint: Some("pom=111|jdtls=1.38.0".to_string()),
        });

        assert_eq!(
            response.workspace_key,
            workspace_key(
                Path::new("/workspace/project"),
                Some("pom=111|jdtls=1.38.0")
            )
        );
    }

    #[test]
    fn non_java_start_is_a_generic_noop() {
        let mut context = java_start_context();
        context.provider_id = "rust".to_string();
        let adapted = adapt_start(&context);

        assert_eq!(adapted.arguments, context.arguments);
        assert_eq!(adapted.data_directory, None);
        assert!(workspace_configuration("rust", &[], JdtSettings::default()).is_none());
        assert!(initialized_notification("rust", JdtSettings::default()).is_none());
        assert!(virtual_source_resolve_params("rust", "jdt://contents/A.class").is_none());
        assert_eq!(
            adapt_initialization_options(
                "rust",
                Some(json!({ "custom": true })),
                &[],
                JdtSettings::default()
            ),
            Some(json!({ "custom": true }))
        );
        let location = ProviderLocation {
            uri: "jdt://contents/A.class".to_string(),
            is_read_only: false,
            display_path: None,
        };
        assert_eq!(normalize_location("rust", location.clone()), location);
    }

    #[test]
    fn java_initialization_enables_class_file_content_without_losing_catalog_options() {
        let options = adapt_initialization_options(
            "JAVA",
            Some(json!({
                "workspace": { "custom": true },
                "bundles": ["/plugins/custom.jar"],
                "extendedClientCapabilities": {
                    "customCapability": true,
                    "classFileContentsSupport": false
                }
            })),
            &[
                PathBuf::from("/jdtls/java-debug/com.microsoft.java.debug.plugin-0.53.1.jar"),
                PathBuf::from(
                    "/jdtls/java-test/extensions/com.microsoft.java.test.plugin-0.42.0.jar",
                ),
            ],
            JdtSettings::default(),
        )
        .unwrap();

        assert_eq!(options["workspace"]["custom"], true);
        assert_eq!(
            options["extendedClientCapabilities"]["customCapability"],
            true
        );
        assert_eq!(
            options["extendedClientCapabilities"]["classFileContentsSupport"],
            true
        );
        assert_eq!(
            options["bundles"],
            json!([
                "/plugins/custom.jar",
                "/jdtls/java-debug/com.microsoft.java.debug.plugin-0.53.1.jar",
                "/jdtls/java-test/extensions/com.microsoft.java.test.plugin-0.42.0.jar"
            ])
        );
    }

    #[test]
    fn java_completion_leaves_arguments_empty_across_configuration_channels() {
        // Initialization, notifications and pull configuration must agree, or
        // JDT LS can restore parameter placeholders after the first completion.
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../../fixtures/lsp/java-completion-v1.json"
        ))
        .expect("Java completion fixture should be valid");
        let expected = &fixture["javaSettings"]["completion"];
        let settings = JdtSettings {
            completion: Some(&fixture["pluginCompletion"]),
            ..Default::default()
        };
        let options = adapt_initialization_options(
            "java",
            Some(json!({ "settings": { "java": { "completion": fixture["pluginCompletion"] } } })),
            &[],
            settings,
        )
        .unwrap();
        assert_eq!(
            options["settings"]["java"]["completion"]["guessMethodArguments"],
            expected["guessMethodArguments"]
        );
        assert_eq!(
            options["settings"]["java"]["completion"]["favoriteStaticMembers"],
            json!(["java.util.Objects.*"])
        );

        assert_eq!(options["settings"]["java"]["completion"], *expected);
        let notification = initialized_notification("java", settings).unwrap();
        assert_eq!(
            notification.params["settings"]["java"]["completion"],
            *expected
        );
        let items = fixture["workspaceConfigurationSections"]
            .as_array()
            .unwrap()
            .iter()
            .map(|section| WorkspaceConfigurationItem {
                scope_uri: None,
                section: Some(section.as_str().unwrap().to_string()),
            })
            .collect::<Vec<_>>();
        let values = workspace_configuration("java", &items, settings).unwrap();
        assert_eq!(values[0]["completion"], *expected);
        assert_eq!(values[1], *expected);
        assert_eq!(values[2], expected["guessMethodArguments"]);
        assert_eq!(values[3], expected["favoriteStaticMembers"]);
        assert_eq!(values[4], expected["filteredTypes"]);
        assert_eq!(values[5], Value::Null);
        assert_eq!(
            fixture["pluginCompletion"]["guessMethodArguments"],
            "insertParameterNames"
        );
    }

    #[test]
    fn java_initialization_options_carry_settings_without_contributions() {
        let options =
            adapt_initialization_options("java", None, &[], JdtSettings::default()).unwrap();

        assert_eq!(
            options["settings"]["java"]["maven"]["downloadSources"],
            false
        );
        // Without a contribution JDT LS keeps its own Maven defaults rather
        // than receiving an empty or null settings section.
        assert!(options["settings"]["java"]["configuration"]
            .get("maven")
            .is_none());
    }

    #[test]
    fn java_initialization_options_omit_import_without_contributions() {
        let options =
            adapt_initialization_options("java", None, &[], JdtSettings::default()).unwrap();

        assert!(options["settings"]["java"].get("import").is_none());
    }

    #[test]
    fn java_workspace_configuration_matches_each_section_shape() {
        let items = [
            "java",
            "java.eclipse.downloadSources",
            "java.maven.downloadSources",
            "java.inlayHints",
            "java.inlayHints.parameterNames",
            "java.inlayHints.parameterNames.enabled",
            "java.implementationsCodeLens",
            "java.implementationsCodeLens.enabled",
            "java.referencesCodeLens",
            "java.referencesCodeLens.enabled",
            "java.unknown",
        ]
        .map(|section| WorkspaceConfigurationItem {
            scope_uri: Some("file:///workspace/project".to_string()),
            section: Some(section.to_string()),
        });
        let values = workspace_configuration("java", &items, JdtSettings::default()).unwrap();

        assert_eq!(values[0]["inlayHints"]["parameterNames"]["enabled"], "all");
        assert_eq!(values[0]["eclipse"]["downloadSources"], false);
        assert_eq!(values[0]["maven"]["downloadSources"], false);
        assert_eq!(values[0]["implementationsCodeLens"]["enabled"], true);
        assert_eq!(values[0]["referencesCodeLens"]["enabled"], true);
        assert_eq!(values[1], false); // java.eclipse.downloadSources
        assert_eq!(values[2], false); // java.maven.downloadSources
        assert_eq!(values[3]["parameterNames"]["enabled"], "all"); // java.inlayHints
        assert_eq!(values[4]["enabled"], "all"); // java.inlayHints.parameterNames
        assert_eq!(values[5], "all"); // java.inlayHints.parameterNames.enabled
        assert_eq!(values[6]["enabled"], true); // java.implementationsCodeLens
        assert_eq!(values[7], true); // java.implementationsCodeLens.enabled
        assert_eq!(values[8]["enabled"], true); // java.referencesCodeLens
        assert_eq!(values[9], true); // java.referencesCodeLens.enabled
        assert_eq!(values[10], Value::Null); // java.unknown
    }

    #[test]
    fn java_initialized_notification_publishes_inlay_settings() {
        let notification = initialized_notification("JAVA", JdtSettings::default()).unwrap();

        assert_eq!(notification.method, "workspace/didChangeConfiguration");
        assert_eq!(
            notification.params["settings"]["java"]["inlayHints"]["parameterNames"]["enabled"],
            "all"
        );
        assert_eq!(
            notification.params["settings"]["java"]["eclipse"]["downloadSources"],
            false
        );
        assert_eq!(
            notification.params["settings"]["java"]["maven"]["downloadSources"],
            false
        );
        assert_eq!(
            notification.params["settings"]["java"]["implementationsCodeLens"]["enabled"],
            true
        );
        assert_eq!(
            notification.params["settings"]["java"]["referencesCodeLens"]["enabled"],
            true
        );
    }

    #[test]
    fn jdt_location_is_read_only_with_a_source_display_path() {
        let location = normalize_location(
            "java",
            ProviderLocation {
                uri: "jdt://contents/java.base/java/util/Map%24Entry.class?=demo".to_string(),
                is_read_only: false,
                display_path: None,
            },
        );

        assert!(location.is_read_only);
        assert_eq!(
            location.display_path.as_deref(),
            Some("java.base/java/util/Map$Entry.java")
        );

        let unchanged = normalize_location(
            "java",
            ProviderLocation {
                uri: "file:///workspace/Main.java".to_string(),
                is_read_only: false,
                display_path: Some("Main.java".to_string()),
            },
        );
        assert!(!unchanged.is_read_only);
        assert_eq!(unchanged.display_path.as_deref(), Some("Main.java"));
    }

    #[test]
    fn virtual_source_uses_java_decompile_execute_command_params() {
        let uri = "jdt://contents/java.base/java/lang/String.class";
        let params = virtual_source_resolve_params("java", uri).unwrap();
        let encoded = serde_json::to_value(params).unwrap();

        assert_eq!(encoded["command"], "java.decompile");
        assert_eq!(encoded["arguments"], json!([uri]));
        assert!(virtual_source_resolve_params("java", "file:///tmp/String.java").is_none());
    }

    #[test]
    fn virtual_source_content_accepts_supported_jdt_result_shapes() {
        assert_eq!(
            virtual_source_content("java", &json!("class String {}")),
            Some("class String {}".to_string())
        );
        assert_eq!(
            virtual_source_content("java", &json!({ "content": "class Object {}" })),
            Some("class Object {}".to_string())
        );
        assert!(virtual_source_content("go", &json!("class String {}")).is_none());
        assert!(virtual_source_content("java", &Value::Null).is_none());
    }
}

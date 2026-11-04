//! Toolchain requirements and version comparison used by JDK selection.
//!
//! Copied from `rust/lithe-core/src/execution/configuration.rs`. The document
//! types and version helpers are unchanged. Reading the requirements of a
//! workspace stays with the host: they come from its generated run
//! configuration layer or from build files (`pom.xml`, `.java-version`, the
//! Maven wrapper) that the Maven and Gradle plugins own.

use crate::protocol::{CoreError, ErrorCode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Version of the run-configuration sidecar documents this crate understands.
const SIDECAR_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Versioned requirements written separately from run configurations.
pub struct ToolchainRequirementsDocument {
    /// Sidecar document version; only [`SIDECAR_VERSION`] is accepted.
    pub version: u32,
    /// Requirements keyed by toolchain role.
    #[serde(default)]
    pub toolchains: BTreeMap<String, ToolchainRequirement>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Constraints the host uses when selecting one toolchain candidate.
pub struct ToolchainRequirement {
    /// Toolchain role, such as `java`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Lowest acceptable version.
    #[serde(default)]
    pub minimum_version: Option<String>,
    /// Preferred distribution vendor.
    #[serde(default)]
    pub preferred_vendor: Option<String>,
    /// Wrapper that pins the toolchain, such as `mvnw`.
    #[serde(default)]
    pub wrapper: Option<String>,
    /// Exact version prefix the toolchain must match.
    #[serde(default)]
    pub version: Option<String>,
    /// Java version a non-Java toolchain needs.
    #[serde(default)]
    pub java: Option<String>,
}

/// Rejects a sidecar document written by another format version.
pub(super) fn validate_sidecar_version(version: u32) -> Result<(), CoreError> {
    if version != SIDECAR_VERSION {
        return Err(
            CoreError::new(ErrorCode::NotSupported, "Unsupported document version")
                .with_details(format!("expected {SIDECAR_VERSION}, found {version}")),
        );
    }
    Ok(())
}

/// Toolchain requirements of the workspace at `root`.
///
/// Rust Core read them from the generated run-configuration layer and fell
/// back to detecting `pom.xml`, `.java-version` and the Maven wrapper. Both
/// sources belong to the host and to the build-tool plugins, so the plugin
/// only accepts requirements the host passes in the request.
// TODO(host-api: API-35 cross-plugin function): the Maven and Gradle plugins
// own the build import, so they export the function that returns the
// workspace's toolchain requirements document, and this plugin calls it
// through the host instead of reading build files itself. The selection
// request then carries that document instead of a workspace root.
pub(super) fn read_requirements(
    root: &Path,
) -> Result<Option<ToolchainRequirementsDocument>, CoreError> {
    if !root.is_dir() {
        return Ok(None);
    }
    Err(CoreError::new(
        ErrorCode::NotSupported,
        "Reading toolchain requirements from a workspace root needs the host",
    )
    .with_details("Pass the requirements document in the request"))
}

/// Whether `actual` satisfies `required` for a toolchain of `kind`.
///
/// `minimum` compares as a lower bound; otherwise `required` is a prefix.
pub(super) fn version_satisfies(kind: &str, actual: &str, required: &str, minimum: bool) -> bool {
    let mut actual_parts = version_parts(actual);
    let mut required_parts = version_parts(required);
    if kind == "java" {
        actual_parts = java_feature_version_parts(actual_parts);
        required_parts = java_feature_version_parts(required_parts);
    }
    if actual_parts.is_empty() || required_parts.is_empty() {
        return false;
    }
    if minimum {
        actual_parts >= required_parts
    } else {
        actual_parts.starts_with(&required_parts)
    }
}

/// Numeric components of a version string, ignoring every separator.
pub(super) fn version_parts(value: &str) -> Vec<u32> {
    value
        .split(|character: char| !character.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect()
}

/// Drops the legacy `1.` prefix Java 8 and earlier report (`1.8.0_504`), so
/// those runtimes compare on the same feature-version scale as `8` or `17.0.12`.
pub(super) fn java_feature_version_parts(parts: Vec<u32>) -> Vec<u32> {
    if parts.len() > 1 && parts[0] == 1 {
        parts[1..].to_vec()
    } else {
        parts
    }
}

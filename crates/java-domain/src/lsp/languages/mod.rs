//! Java-specific language-server rules moved from Rust Core's
//! `lsp/languages` module.

pub mod java_entrypoints;
pub mod java_launch;
pub mod java_main_methods;
pub mod java_navigation_syntax;
mod java_run_markers;
pub mod java_tests;
mod java_workspace;
pub mod jdt;
pub mod jdt_build;
mod jdt_configuration;
pub mod jdt_contributions;
pub mod jdt_navigation;
pub mod jdt_progress;
mod jdt_project_metadata;
pub mod project_preparation;

pub use java_run_markers::{
    java_run_markers, JavaRunMarker, JavaRunMarkerKind, JavaRunMarkerStatus, JavaRunMarkersRequest,
    JavaRunMarkersResponse, MainMethodInput, RangeInput, TestItemInput,
};
pub use java_workspace::{
    java_workspace_policy, jdt_cache_retention, jdt_workspace_fingerprint,
    JavaWorkspacePolicyRequest, JdtBuildFileObservation, JdtCacheEntry, JdtCacheRetentionRequest,
    JdtCacheRetentionResponse, JdtWorkspaceFingerprintRequest, JdtWorkspaceFingerprintResponse,
};
pub use jdt::{resolve_workspace_key, workspace_key, JdtWorkspaceKeyRequest};
pub use jdt_build::JavaBuildReport;
pub use jdt_configuration::prepare_configuration_area as prepare_jdt_configuration_area;
pub use jdt_project_metadata::{
    prepare_workspace as prepare_jdt_workspace,
    prepare_workspace_with as prepare_jdt_workspace_with,
};

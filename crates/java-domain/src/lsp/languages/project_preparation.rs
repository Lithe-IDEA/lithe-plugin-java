//! User-facing Java preparation state derived from the existing session and build owner.

use super::jdt::ContributedSettingsStatus;
use crate::lsp::LspLifecycleState;
use serde::Serialize;

/// Stable presentation state. Ready means project preparation, not compilation, completed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPreparation {
    /// Current stage: starting, importing, configuring, building, ready, or stopped.
    pub phase: &'static str,
    /// Presentation outcome: idle, loading, ready, or failed.
    pub status: &'static str,
    /// Whether Java launch must wait for the session-wide preparation gate.
    pub blocks_run: bool,
}

/// Projects upstream lifecycle facts without starting work or treating indexing as a gate.
///
/// `contributed` is the state of the settings that build-system plugins
/// contribute; while it is pending the projects are still being configured.
pub fn snapshot(
    lifecycle: LspLifecycleState,
    contributed: ContributedSettingsStatus,
    configuring: bool,
    building: bool,
) -> ProjectPreparation {
    use LspLifecycleState::*;
    let (phase, status) = match lifecycle {
        Created | ProcessStarting => ("starting", "loading"),
        Initializing => ("importing", "loading"),
        Failed => ("starting", "failed"),
        Stopping | Stopped => ("stopped", "idle"),
        Ready => match contributed {
            ContributedSettingsStatus::Pending => ("configuring", "loading"),
            _ if configuring => ("configuring", "loading"),
            _ if building => ("building", "loading"),
            ContributedSettingsStatus::Failed => ("configuring", "failed"),
            ContributedSettingsStatus::Settled => ("ready", "ready"),
        },
    };
    ProjectPreparation {
        phase,
        status,
        // A missing contribution leaves the session on default settings.
        // Keep the failure visible, but let JDT validate the selected target
        // rather than blocking modules the contribution did not affect.
        blocks_run: lifecycle != LspLifecycleState::Ready || status == "loading",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ContributedSettingsStatus::{Failed, Pending, Settled};

    #[test]
    fn serialized_snapshots_match_the_shared_fixture() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../fixtures/lsp/project-preparation-v1.json"
        ))
        .unwrap();
        let cases = [
            (LspLifecycleState::Created, Settled, false, false),
            (LspLifecycleState::Initializing, Settled, false, false),
            (LspLifecycleState::Ready, Pending, false, false),
            (LspLifecycleState::Ready, Settled, false, true),
            (LspLifecycleState::Ready, Settled, false, false),
            (LspLifecycleState::Ready, Failed, false, false),
            (LspLifecycleState::Stopped, Settled, false, false),
        ];
        let snapshots = fixture["snapshots"].as_array().unwrap();
        assert_eq!(snapshots.len(), cases.len());
        for ((lifecycle, contributed, configuring, building), expected) in
            cases.into_iter().zip(snapshots)
        {
            assert_eq!(
                &serde_json::to_value(snapshot(lifecycle, contributed, configuring, building))
                    .unwrap(),
                expected
            );
        }
    }

    #[test]
    fn service_ready_does_not_bypass_project_configuration() {
        let ready = LspLifecycleState::Ready;
        assert_eq!(snapshot(ready, Pending, false, false).phase, "configuring");
        assert!(snapshot(ready, Pending, false, false).blocks_run);
        assert!(snapshot(ready, Settled, true, false).blocks_run);
        assert_eq!(snapshot(ready, Settled, false, true).phase, "building");
        assert!(!snapshot(ready, Settled, false, false).blocks_run);
    }

    #[test]
    fn failures_and_stopped_sessions_never_report_ready() {
        let ready = LspLifecycleState::Ready;
        assert!(snapshot(ready, Failed, true, false).blocks_run);
        assert!(snapshot(ready, Failed, false, true).blocks_run);
        let failed = snapshot(ready, Failed, false, false);
        assert_eq!((failed.phase, failed.status), ("configuring", "failed"));
        assert!(!failed.blocks_run);
        assert!(snapshot(LspLifecycleState::Stopped, Settled, false, false).blocks_run);
        assert_eq!(
            snapshot(LspLifecycleState::Initializing, Settled, false, false).phase,
            "importing"
        );
        assert_eq!(
            snapshot(LspLifecycleState::Failed, Settled, false, false).status,
            "failed"
        );
    }
}

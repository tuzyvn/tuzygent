use serde::{Deserialize, Serialize};
use std::fmt;

/// Snapshot-qualified target reference (`@<snapshot_id>:<target_id>`).
///
/// Prevents blind actions against bare or stale element/resource IDs by binding
/// every mutating tool call to an observed snapshot version.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SnapshotRef {
    pub snapshot_id: String,
    pub target_id: String,
}

impl SnapshotRef {
    /// Creates a new `SnapshotRef` from non-empty snapshot and target identifiers.
    pub fn new(
        snapshot_id: impl Into<String>,
        target_id: impl Into<String>,
    ) -> Result<Self, String> {
        let snapshot_id = snapshot_id.into();
        let target_id = target_id.into();
        if snapshot_id.trim().is_empty() || snapshot_id.contains(':') || snapshot_id.contains(char::is_whitespace) {
            return Err(format!(
                "Invalid snapshot_id '{}' in SnapshotRef: must be non-empty without ':' or whitespace",
                snapshot_id
            ));
        }
        if target_id.trim().is_empty() {
            return Err("Invalid target_id in SnapshotRef: must be non-empty".to_string());
        }
        Ok(Self {
            snapshot_id,
            target_id,
        })
    }

    /// Parses a qualified ref string formatted as `@<snapshot_id>:<target_id>`.
    ///
    /// Bare refs such as `@e8` (missing snapshot scope) are rejected.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let Some(rest) = raw.strip_prefix('@') else {
            return Err(format!(
                "INVALID_ARGS: Ref '{}' must start with '@' and follow '@<snapshot_id>:<target_id>'",
                raw
            ));
        };
        let Some((snapshot_id, target_id)) = rest.split_once(':') else {
            return Err(format!(
                "INVALID_ARGS: Bare ref '{}' is missing snapshot scope; expected '@<snapshot_id>:<target_id>'",
                raw
            ));
        };
        Self::new(snapshot_id, target_id)
    }
}

impl fmt::Display for SnapshotRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "@{}:{}", self.snapshot_id, self.target_id)
    }
}

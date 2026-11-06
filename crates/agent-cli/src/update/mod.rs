pub mod benchmark;
pub mod resilience;
pub mod sharded;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::fault::CliExecutionError;

pub use benchmark::{UpdateBenchmarkHarness, UpdateBenchmarkStats};
pub use resilience::ResilientUpdater;
pub use sharded::ShardedResilientUpdater;

pub const DEFAULT_REPOSITORY: &str = "https://github.com/tuzyvn/tuzygent.git";
pub const DEFAULT_BRANCH: &str = "main";

/// Ephemeral staging directory that self-cleans on drop
pub struct TemporaryDirectory(pub PathBuf);

impl TemporaryDirectory {
    pub fn new(parent: &Path) -> Result<Self, CliExecutionError> {
        let path = parent.join(format!(".agent-ctl-update-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).map_err(|e| {
            CliExecutionError::TransientIo(format!("Cannot create update staging directory: {e}"))
        })?;
        Ok(Self(path))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    AlreadyUpToDate { current: String },
    UpdateAvailable { current: String, latest: String },
    Updated { from: String, to: String },
}

pub(crate) fn checked_command(
    command: &mut Command,
    description: &str,
) -> Result<(), CliExecutionError> {
    let output = command.output().map_err(|e| {
        CliExecutionError::TransientIo(format!("{description} failed to spawn: {e}"))
    })?;
    if !output.status.success() {
        let code = output.status.code().unwrap_or(1);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = format!("{description} exited with status {code}: {stderr} {stdout}");
        return Err(CliExecutionError::NonZeroExit {
            code,
            message: detail.trim().to_string(),
        });
    }
    Ok(())
}

pub fn run(check: bool) -> anyhow::Result<()> {
    let mut updater = ResilientUpdater::default();
    updater.run_sync(check).map_err(|e| anyhow::anyhow!(e))?;
    Ok(())
}

pub async fn run_resilient_command(check: bool) -> Result<UpdateStatus, CliExecutionError> {
    let mut updater = ResilientUpdater::default();
    updater.run_resilient(check).await
}

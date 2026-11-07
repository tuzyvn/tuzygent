use crate::fault::CliExecutionError;

use super::super::{TemporaryDirectory, UpdateStatus};
use super::ResilientUpdater;

impl ResilientUpdater {
    pub fn run_sync(&mut self, check: bool) -> Result<UpdateStatus, CliExecutionError> {
        let temporary = TemporaryDirectory::new(&std::env::temp_dir())?;
        let source = temporary.path().join("source");
        println!("Checking {} ({})...", self.repository_url, self.branch);
        let current = env!("AGENTCTL_COMMIT");
        if let Err(error) = self.fetch_repository(&source) {
            println!("Installed: {current}\nLatest:    unavailable");
            if check {
                return Ok(UpdateStatus::LatestUnavailable {
                    current: current.to_string(),
                });
            }
            return Err(error);
        }
        let latest = match self.check_remote_commit(&source) {
            Ok(latest) => latest,
            Err(error) => {
                println!("Installed: {current}\nLatest:    unavailable");
                if check {
                    return Ok(UpdateStatus::LatestUnavailable {
                        current: current.to_string(),
                    });
                }
                return Err(error);
            }
        };
        println!("Installed: {current}\nLatest:    {latest}");
        if current == latest {
            println!("Already up to date.");
            return Ok(UpdateStatus::AlreadyUpToDate {
                current: current.to_string(),
            });
        }
        if check {
            println!("Update available. Run: agent-ctl update");
            return Ok(UpdateStatus::UpdateAvailable {
                current: current.to_string(),
                latest,
            });
        }
        let destination = match self.destination.as_ref() {
            Some(d) => d.clone(),
            None => std::env::current_exe().map_err(|e| {
                CliExecutionError::Other(format!("Cannot locate current executable: {e}"))
            })?,
        };
        let _write_probe =
            TemporaryDirectory::new(destination.parent().ok_or_else(|| {
                CliExecutionError::Other("Missing install directory".to_string())
            })?)?;

        println!("Building {latest} in release mode (requires Cargo and a Rust toolchain)...");
        let target = temporary.path().join("target");
        self.build_release(&source, &target)?;
        self.install_candidate(&target.join("release/agent-ctl"), &destination, &latest)?;
        Ok(UpdateStatus::Updated {
            from: current.to_string(),
            to: latest,
        })
    }

    pub async fn run_resilient(&mut self, check: bool) -> Result<UpdateStatus, CliExecutionError> {
        self.run_sync(check)
    }
}

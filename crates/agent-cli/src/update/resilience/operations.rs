use std::fs;
use std::future::Future;
use std::path::Path;
use std::process::Command;

use crate::fault::CliExecutionError;

use super::super::{checked_command, TemporaryDirectory};
use super::ResilientUpdater;

impl ResilientUpdater {
    pub fn execute_step<F, T>(&mut self, step_name: &str, op: F) -> Result<T, CliExecutionError>
    where
        F: FnMut() -> Result<T, CliExecutionError>,
    {
        self.runner.run_resilient_sync(step_name, op)
    }

    pub async fn execute_operation<F, Fut, T>(
        &mut self,
        action_name: &str,
        op: F,
    ) -> Result<T, CliExecutionError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, CliExecutionError>>,
    {
        self.runner.run_resilient(action_name, op).await
    }

    pub async fn execute_batch_operations<T: Clone>(
        &mut self,
        items: &[(&str, Result<T, CliExecutionError>)],
    ) -> Vec<Result<T, CliExecutionError>> {
        let mut results = Vec::with_capacity(items.len());
        for (action, res) in items {
            let res_clone = res.clone();
            let out = self
                .runner
                .run_resilient(action, || {
                    let r = res_clone.clone();
                    async move { r }
                })
                .await;
            results.push(out);
        }
        results
    }

    pub fn fetch_repository(&mut self, source: &Path) -> Result<(), CliExecutionError> {
        let repo_url = self.repository_url.clone();
        let branch = self.branch.clone();
        self.runner.run_resilient_sync("update_git_clone", || {
            if source.exists() {
                let _ = fs::remove_dir_all(source);
            }
            let mut cmd = Command::new("git");
            cmd.args([
                "clone",
                "--quiet",
                "--depth",
                "1",
                "--single-branch",
                "--branch",
                &branch,
                "--",
                &repo_url,
            ])
            .arg(source);
            checked_command(
                &mut cmd,
                "Cannot fetch AgentCore; Git and network access are required",
            )
        })
    }

    pub fn check_remote_commit(&mut self, source: &Path) -> Result<String, CliExecutionError> {
        self.runner.run_resilient_sync("update_git_rev_parse", || {
            let output = Command::new("git")
                .current_dir(source)
                .args(["rev-parse", "--short", "HEAD"])
                .output()
                .map_err(|e| {
                    CliExecutionError::TransientIo(format!("Cannot execute git rev-parse: {e}"))
                })?;
            if !output.status.success() {
                let code = output.status.code().unwrap_or(1);
                return Err(CliExecutionError::NonZeroExit {
                    code,
                    message: "Cannot read fetched source commit".to_string(),
                });
            }
            let latest = String::from_utf8(output.stdout)
                .map_err(|e| CliExecutionError::Other(format!("Invalid git output utf8: {e}")))?
                .trim()
                .to_string();
            Ok(latest)
        })
    }

    pub fn build_release(&mut self, source: &Path, target: &Path) -> Result<(), CliExecutionError> {
        self.runner.run_resilient_sync("update_cargo_build", || {
            let mut cmd = Command::new("cargo");
            cmd.current_dir(source)
                .args([
                    "build",
                    "--locked",
                    "--release",
                    "-p",
                    "agent-cli",
                    "--bin",
                    "agent-ctl",
                    "--target-dir",
                ])
                .arg(target)
                .env_remove("CARGO_BUILD_TARGET")
                .env_remove("AGENTCTL_RELEASE_DATE");
            checked_command(&mut cmd, "Cannot build AgentCore")
        })
    }

    pub fn install_candidate(
        &mut self,
        candidate: &Path,
        destination: &Path,
        expected_commit: &str,
    ) -> Result<(), CliExecutionError> {
        self.runner.run_resilient_sync("update_install_stage", || {
            let parent = destination.parent().ok_or_else(|| {
                CliExecutionError::Other("Executable has no parent directory".to_string())
            })?;
            let staging = TemporaryDirectory::new(parent)?;
            let staged = staging.path().join("agent-ctl");
            fs::copy(candidate, &staged).map_err(|e| {
                CliExecutionError::TransientIo(format!("Cannot stage candidate executable: {e}"))
            })?;

            let output = Command::new(&staged)
                .arg("--version")
                .output()
                .map_err(|e| {
                    CliExecutionError::TransientIo(format!("Cannot run candidate executable: {e}"))
                })?;

            let version = String::from_utf8_lossy(&output.stdout);
            if !output.status.success()
                || !version.starts_with("agent-ctl ")
                || !version
                    .lines()
                    .any(|line| line == format!("Commit: {expected_commit}"))
            {
                return Err(CliExecutionError::Other(
                    "New executable failed version validation; installed binary was not changed"
                        .to_string(),
                ));
            }

            let file = fs::File::open(&staged).map_err(|e| {
                CliExecutionError::TransientIo(format!("Failed to open staged binary: {e}"))
            })?;
            file.sync_all().map_err(|e| {
                CliExecutionError::TransientIo(format!("Failed to sync staged binary: {e}"))
            })?;
            drop(file);

            fs::rename(&staged, destination).map_err(|e| {
                CliExecutionError::TransientIo(format!(
                    "Cannot replace installed executable; check permissions: {e}"
                ))
            })?;

            print!("Updated: {version}");
            Ok(())
        })
    }
}

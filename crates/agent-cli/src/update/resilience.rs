use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::fault::{CliExecutionError, CliFaultPolicy};
use crate::resilience::ResilientCliRunner;

use super::{
    checked_command, TemporaryDirectory, UpdateStatus, DEFAULT_BRANCH, DEFAULT_REPOSITORY,
};

#[derive(Clone)]
pub struct ResilientUpdater {
    repository_url: String,
    branch: String,
    destination: Option<PathBuf>,
    runner: ResilientCliRunner,
}

impl Default for ResilientUpdater {
    fn default() -> Self {
        Self::new(ResilientCliRunner::default())
    }
}

impl ResilientUpdater {
    pub fn new(runner: ResilientCliRunner) -> Self {
        Self {
            repository_url: DEFAULT_REPOSITORY.to_string(),
            branch: DEFAULT_BRANCH.to_string(),
            destination: None,
            runner,
        }
    }

    pub fn with_policy(policy: CliFaultPolicy, max_retries: usize) -> Self {
        Self::new(ResilientCliRunner::new(policy, max_retries))
    }

    pub fn with_audit_workspace(mut self, path: impl AsRef<Path>) -> Self {
        self.runner = self.runner.with_audit_workspace(path);
        self
    }

    pub fn with_repository(mut self, repo: impl Into<String>) -> Self {
        self.repository_url = repo.into();
        self
    }

    pub fn with_branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = branch.into();
        self
    }

    pub fn with_destination(mut self, dest: impl Into<PathBuf>) -> Self {
        self.destination = Some(dest.into());
        self
    }

    pub fn with_max_capacity(mut self, max: usize) -> Self {
        self.runner = self.runner.with_max_capacity(max);
        self
    }

    pub fn max_capacity(&self) -> Option<usize> {
        self.runner.max_capacity()
    }

    pub fn memory_usage_bytes(&self) -> usize {
        self.runner.memory_usage_bytes()
    }

    pub fn runner(&self) -> &ResilientCliRunner {
        &self.runner
    }

    pub fn runner_mut(&mut self) -> &mut ResilientCliRunner {
        &mut self.runner
    }

    pub fn recovered_count(&self) -> usize {
        self.runner.recovered_count()
    }

    pub fn total_executed(&self) -> usize {
        self.runner.total_executed()
    }

    pub fn failed_count(&self) -> usize {
        self.runner.failed_count()
    }

    pub fn rejected_count(&self) -> usize {
        self.runner.rejected_count()
    }

    pub fn panics_caught_count(&self) -> usize {
        self.runner.panics_caught_count()
    }

    pub fn reset_counts(&mut self) {
        self.runner.reset_counts();
    }

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
            checked_command(&mut cmd, "Cannot fetch AgentCore; Git and network access are required")
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

            let output = Command::new(&staged).arg("--version").output().map_err(|e| {
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

    pub fn run_sync(&mut self, check: bool) -> Result<UpdateStatus, CliExecutionError> {
        let temporary = TemporaryDirectory::new(&std::env::temp_dir())?;
        let source = temporary.path().join("source");
        println!("Checking {} ({})...", self.repository_url, self.branch);
        self.fetch_repository(&source)?;
        let latest = self.check_remote_commit(&source)?;
        let current = env!("AGENTCTL_COMMIT");
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
        let _write_probe = TemporaryDirectory::new(
            destination
                .parent()
                .ok_or_else(|| CliExecutionError::Other("Missing install directory".to_string()))?,
        )?;

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

use std::path::{Path, PathBuf};

use crate::fault::CliFaultPolicy;
use crate::resilience::ResilientCliRunner;

use super::super::{DEFAULT_BRANCH, DEFAULT_REPOSITORY};

#[derive(Clone)]
pub struct ResilientUpdater {
    pub(super) repository_url: String,
    pub(super) branch: String,
    pub(super) destination: Option<PathBuf>,
    pub(super) runner: ResilientCliRunner,
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
}

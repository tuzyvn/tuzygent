mod aggregate;
mod checks;
mod format;
mod parser;
#[cfg(test)]
mod tests;

use std::path::Path;

pub struct TaskVerifier<'a> {
    workspace_dir: &'a Path,
    sandbox: crate::sandbox::SandboxConfig,
    guardrail: agent_domain::GuardrailPolicy,
}

impl<'a> TaskVerifier<'a> {
    pub fn new(workspace_dir: &'a Path) -> Self {
        Self {
            workspace_dir,
            sandbox: Default::default(),
            guardrail: agent_domain::GuardrailPolicy::default_enterprise(),
        }
    }

    pub fn with_policy(
        workspace_dir: &'a Path,
        sandbox: crate::sandbox::SandboxConfig,
        guardrail: agent_domain::GuardrailPolicy,
    ) -> Self {
        Self {
            workspace_dir,
            sandbox,
            guardrail,
        }
    }
}

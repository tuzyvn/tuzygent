pub mod command;
pub mod config;
pub mod context;
pub mod driver;
pub mod env;
pub mod self_check;
pub mod workspace;

pub use command::{command_sandbox, expand_home, wrap_command, CommandManager};
pub use self_check::{
    filter_sandbox_env_whitelist, is_sandbox_env_allowed, verify_sandbox_plan, SandboxMutation,
    SandboxSelfCheckReport, SANDBOX_CPU_TOPOLOGY_PATHS, SANDBOX_ENV_NEVER_LIST,
};
pub use config::{IsolationMode, SandboxConfig, SandboxSpec};
pub use context::{inject_agent_brief, purge_sidecar_context, SidecarContextManager};
pub use driver::SandboxDriverKind;
pub use env::SandboxEnvironment;
pub use workspace::{
    is_retryable_workspace_error, prepare_workspace, teardown_workspace, ResilientWorkspaceManager,
    WorkspaceAuditRecord, WorkspaceBenchmarkHarness, WorkspaceBenchmarkStats, WorkspaceError,
    WorkspaceFaultKind, WorkspaceFaultPolicy, WorkspaceManager,
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_driver_detection() {
        let driver = SandboxConfig::detect_best_driver();
        assert!(
            driver == SandboxDriverKind::Bubblewrap
                || driver == SandboxDriverKind::Docker
                || driver == SandboxDriverKind::Host
        );
    }

    #[test]
    fn test_bwrap_wrap_command() {
        let config = SandboxConfig {
            driver: SandboxDriverKind::Bubblewrap,
            ..Default::default()
        };
        let env = SandboxEnvironment::with_config(".", config);

        let (bin, args) = env.wrap_command("ls", &["-la".to_string()], Path::new("/tmp"));
        assert_eq!(bin, "bwrap");
        assert!(args.contains(&"--ro-bind".to_string()));
        assert!(args.contains(&"ls".to_string()));
        assert!(args.contains(&"-la".to_string()));
    }

    #[test]
    fn bwrap_includes_public_certificate_stores() {
        let environment = SandboxEnvironment::with_config(
            ".",
            SandboxConfig {
                driver: SandboxDriverKind::Bubblewrap,
                ..Default::default()
            },
        );
        let (_, args) = environment.wrap_command("true", &[], Path::new("/tmp"));
        for store in ["/etc/ssl", "/etc/ca-certificates", "/etc/pki"] {
            if Path::new(store).exists() {
                assert!(
                    args.windows(3)
                        .any(|args| args == ["--ro-bind", store, store]),
                    "missing CA store {store}"
                );
            }
        }
        assert!(!args.windows(3).any(|args| args == ["--ro-bind", "/", "/"]));
    }
}

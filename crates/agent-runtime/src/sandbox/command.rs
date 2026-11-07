use crate::sandbox::config::SandboxConfig;
use crate::sandbox::driver::SandboxDriverKind;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Helper to expand leading '~' to the user's home directory.
pub fn expand_home(path_str: &str) -> PathBuf {
    if path_str.starts_with("~/") || path_str == "~" {
        if let Ok(home) = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
            return PathBuf::from(path_str.replacen('~', &home, 1));
        }
    }
    PathBuf::from(path_str)
}

/// Wrap an OS command with the configured Sandbox driver (Docker, Bubblewrap, or Host).
pub fn wrap_command(
    config: &SandboxConfig,
    original_bin: &str,
    original_args: &[String],
    run_dir: &Path,
) -> (String, Vec<String>) {
    let resolved_dir = std::fs::canonicalize(run_dir).unwrap_or_else(|_| run_dir.to_path_buf());
    let run_dir = resolved_dir.as_path();
    match config.driver {
        SandboxDriverKind::Bubblewrap => {
            let mut bwrap_args = vec![
                "--unshare-pid".to_string(),
                "--unshare-ipc".to_string(),
                "--unshare-uts".to_string(),
                "--die-with-parent".to_string(),
                "--new-session".to_string(),
                "--dev".to_string(),
                "/dev".to_string(),
                "--proc".to_string(),
                "/proc".to_string(),
                "--tmpfs".to_string(),
                "/tmp".to_string(),
            ];

            // Expose runtimes and public system configuration, not the host home/root.
            for path in [
                "/usr",
                "/bin",
                "/sbin",
                "/lib",
                "/lib64",
                "/etc/ssl",
                "/etc/ca-certificates",
                "/etc/pki",
                "/etc/nsswitch.conf",
                "/etc/gai.conf",
                "/etc/host.conf",
                "/etc/resolv.conf",
                "/etc/hosts",
                "/etc/passwd",
                "/etc/group",
            ] {
                if Path::new(path).exists() {
                    bwrap_args.extend(["--ro-bind".into(), path.into(), path.into()]);
                }
            }
            bwrap_args.extend([
                "--dir".into(),
                "/root".into(),
                "--setenv".into(),
                "HOME".into(),
                "/root".into(),
            ]);

            // Mount target working directory (read-write or ephemeral overlay)
            let dir_str = run_dir.to_string_lossy().to_string();
            if config.ephemeral_overlay {
                bwrap_args.push("--ro-bind".to_string());
                bwrap_args.push(dir_str.clone());
                bwrap_args.push(dir_str.clone());
                bwrap_args.push("--overlay-src".to_string());
                bwrap_args.push(dir_str.clone());
                bwrap_args.push("--tmp-overlay".to_string());
                bwrap_args.push(dir_str.clone());
            } else {
                bwrap_args.push("--bind".to_string());
                bwrap_args.push(dir_str.clone());
                bwrap_args.push(dir_str.clone());
            }

            bwrap_args.push("--chdir".to_string());
            bwrap_args.push(dir_str);

            // Mount binary directory if outside standard system paths (~/.local/bin, cargo, etc.)
            let bin_path = Path::new(original_bin);
            if let Some(parent) = bin_path.parent() {
                let parent_str = parent.to_string_lossy().to_string();
                if parent.exists()
                    && !parent_str.starts_with("/usr")
                    && !parent_str.starts_with("/bin")
                    && !parent_str.starts_with("/lib")
                {
                    bwrap_args.push("--ro-bind".to_string());
                    bwrap_args.push(parent_str.clone());
                    bwrap_args.push(parent_str);
                }
            }

            // Mount read-only credentials if they exist
            for mount in &config.ro_credentials {
                let expanded_source = expand_home(&mount.source);
                if expanded_source.exists() {
                    bwrap_args.push("--ro-bind".to_string());
                    bwrap_args.push(expanded_source.to_string_lossy().to_string());
                    bwrap_args.push(mount.target.clone());
                }
            }

            // Inject allowed environment variables
            for env_key in &config.env_whitelist {
                if let Ok(val) = std::env::var(env_key) {
                    bwrap_args.push("--setenv".to_string());
                    bwrap_args.push(env_key.clone());
                    bwrap_args.push(val);
                }
            }

            // Append original command and arguments
            bwrap_args.push(original_bin.to_string());
            bwrap_args.extend_from_slice(original_args);

            ("bwrap".to_string(), bwrap_args)
        }

        SandboxDriverKind::Docker => {
            let mut docker_args = vec![
                "run".to_string(),
                "--rm".to_string(),
                "-i".to_string(),
                "--cap-drop=ALL".to_string(),
                "--security-opt=no-new-privileges".to_string(),
                "--pids-limit=256".to_string(),
            ];

            if config.ephemeral_overlay {
                docker_args.push("-v".to_string());
                docker_args.push(format!("{}:/workspace:ro", run_dir.to_string_lossy()));
                docker_args.push("--tmpfs".to_string());
                docker_args.push("/workspace".to_string());
            } else {
                docker_args.push("-v".to_string());
                docker_args.push(format!("{}:/workspace", run_dir.to_string_lossy()));
            }

            docker_args.push("-w".to_string());
            docker_args.push("/workspace".to_string());

            if let Some(mem) = config.memory_limit_mb {
                docker_args.push("--memory".to_string());
                docker_args.push(format!("{}m", mem));
            }

            for mount in &config.ro_credentials {
                let expanded_source = expand_home(&mount.source);
                if expanded_source.exists() {
                    docker_args.push("-v".to_string());
                    docker_args.push(format!(
                        "{}:{}:ro",
                        expanded_source.to_string_lossy(),
                        mount.target
                    ));
                }
            }

            for env_key in &config.env_whitelist {
                if let Ok(val) = std::env::var(env_key) {
                    docker_args.push("-e".to_string());
                    docker_args.push(format!("{}={}", env_key, val));
                }
            }

            docker_args.push(config.docker_image.clone());
            docker_args.push(original_bin.to_string());
            docker_args.extend_from_slice(original_args);

            ("docker".to_string(), docker_args)
        }

        SandboxDriverKind::Host => (original_bin.to_string(), original_args.to_vec()),
    }
}

/// Generates execution sandbox parameters for process engine.
pub fn command_sandbox(
    config: &SandboxConfig,
    run_dir: &Path,
) -> Option<agent_engine::process::CommandSandbox> {
    if config.driver == SandboxDriverKind::Host {
        return None;
    }
    let (program, mut prefix_args) = wrap_command(config, "__engine__", &[], run_dir);
    prefix_args.pop();
    let container_name = if config.driver == SandboxDriverKind::Docker {
        let name = format!("tuzygent-{}", Uuid::new_v4().simple());
        if !prefix_args.is_empty() {
            prefix_args.splice(1..1, ["--name".into(), name.clone()]);
        } else {
            prefix_args.extend(["--name".into(), name.clone()]);
        }
        Some(name)
    } else {
        None
    };
    Some(agent_engine::process::CommandSandbox {
        program,
        prefix_args,
        container_name,
        env_whitelist: config.env_whitelist.clone(),
        working_dir: if config.driver == SandboxDriverKind::Docker {
            "/workspace".into()
        } else {
            std::fs::canonicalize(run_dir)
                .unwrap_or_else(|_| run_dir.to_path_buf())
                .to_string_lossy()
                .into_owned()
        },
    })
}

/// Object-oriented interface for command sandboxing operations.
#[derive(Debug, Clone)]
pub struct CommandManager {
    pub config: SandboxConfig,
}

impl CommandManager {
    pub fn new(config: SandboxConfig) -> Self {
        Self { config }
    }

    pub fn wrap_command(
        &self,
        original_bin: &str,
        original_args: &[String],
        run_dir: &Path,
    ) -> (String, Vec<String>) {
        wrap_command(&self.config, original_bin, original_args, run_dir)
    }

    pub fn command_sandbox(&self, run_dir: &Path) -> Option<agent_engine::process::CommandSandbox> {
        command_sandbox(&self.config, run_dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bwrap_wrap_command() {
        let config = SandboxConfig {
            driver: SandboxDriverKind::Bubblewrap,
            ..Default::default()
        };
        let (bin, args) = wrap_command(&config, "ls", &["-la".to_string()], Path::new("/tmp"));
        assert_eq!(bin, "bwrap");
        assert!(args.contains(&"--ro-bind".to_string()));
        assert!(args.contains(&"ls".to_string()));
        assert!(args.contains(&"-la".to_string()));
    }

    #[test]
    fn test_bwrap_includes_public_certificate_stores() {
        let config = SandboxConfig {
            driver: SandboxDriverKind::Bubblewrap,
            ..Default::default()
        };
        let (_, args) = wrap_command(&config, "true", &[], Path::new("/tmp"));
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

    #[test]
    fn test_docker_wrap_command() {
        let config = SandboxConfig {
            driver: SandboxDriverKind::Docker,
            docker_image: "test-image:latest".to_string(),
            memory_limit_mb: Some(1024),
            ..Default::default()
        };
        let (bin, args) = wrap_command(&config, "echo", &["hello".to_string()], Path::new("/tmp"));
        assert_eq!(bin, "docker");
        assert_eq!(args[0], "run");
        assert!(args.contains(&"--rm".to_string()));
        assert!(args.windows(2).any(|w| w == ["--memory", "1024m"]));
        assert!(args.windows(2).any(|w| w == ["-w", "/workspace"]));
        assert_eq!(
            args.iter().rev().take(3).collect::<Vec<_>>(),
            vec![
                &"hello".to_string(),
                &"echo".to_string(),
                &"test-image:latest".to_string(),
            ]
        );
    }

    #[test]
    fn test_host_wrap_command() {
        let config = SandboxConfig {
            driver: SandboxDriverKind::Host,
            ..Default::default()
        };
        let (bin, args) = wrap_command(&config, "ls", &["-la".to_string()], Path::new("/tmp"));
        assert_eq!(bin, "ls");
        assert_eq!(args, vec!["-la".to_string()]);
        assert!(command_sandbox(&config, Path::new("/tmp")).is_none());
    }

    #[test]
    fn test_command_manager() {
        let config = SandboxConfig {
            driver: SandboxDriverKind::Host,
            ..Default::default()
        };
        let manager = CommandManager::new(config);
        let (bin, args) = manager.wrap_command("whoami", &[], Path::new("/tmp"));
        assert_eq!(bin, "whoami");
        assert!(args.is_empty());
        assert!(manager.command_sandbox(Path::new("/tmp")).is_none());
    }

    #[test]
    fn test_expand_home_unit() {
        assert_eq!(
            expand_home("/absolute/path"),
            PathBuf::from("/absolute/path")
        );
        assert_eq!(expand_home("rel/path"), PathBuf::from("rel/path"));
    }
}

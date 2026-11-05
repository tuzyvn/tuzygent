//! Command sandbox modeling and process command construction.

use crate::EngineInput;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::process::Command;

/// Trusted, runtime-created wrapper. Never accepted from serialized engine inputs.
#[derive(Debug, Clone)]
pub struct CommandSandbox {
    pub program: String,
    pub prefix_args: Vec<String>,
    pub env_whitelist: Vec<String>,
    pub working_dir: String,
    pub container_name: Option<String>,
}

impl CommandSandbox {
    /// Create a new command sandbox definition with safe defaults.
    pub fn new(
        program: impl Into<String>,
        prefix_args: Vec<String>,
        working_dir: impl Into<String>,
    ) -> Self {
        Self {
            program: program.into(),
            prefix_args,
            env_whitelist: Vec::new(),
            working_dir: working_dir.into(),
            container_name: None,
        }
    }

    /// Whitelist additional environment variable keys to propagate into sandbox.
    pub fn with_env_whitelist(mut self, env_whitelist: Vec<String>) -> Self {
        self.env_whitelist = env_whitelist;
        self
    }

    /// Set an optional container name for containerized execution runtimes.
    pub fn with_container_name(mut self, container_name: impl Into<String>) -> Self {
        self.container_name = Some(container_name.into());
        self
    }

    /// Append prefix arguments to sandbox command.
    pub fn with_prefix_args(mut self, args: Vec<String>) -> Self {
        self.prefix_args.extend(args);
        self
    }

    /// Override the sandbox working directory.
    pub fn with_working_dir(mut self, working_dir: impl Into<String>) -> Self {
        self.working_dir = working_dir.into();
        self
    }

    /// Add a single environment variable to the whitelist.
    pub fn add_env_var(&mut self, key: impl Into<String>) {
        let k = key.into();
        if !self.env_whitelist.contains(&k) {
            self.env_whitelist.push(k);
        }
    }

    /// Check if the sandbox uses bubblewrap.
    pub fn is_bwrap(&self) -> bool {
        self.program == "bwrap"
    }

    /// Check if the sandbox uses docker.
    pub fn is_docker(&self) -> bool {
        self.program == "docker"
    }

    /// Check if the sandbox has any containerization enabled.
    pub fn is_containerized(&self) -> bool {
        self.is_bwrap() || self.is_docker() || self.container_name.is_some()
    }
}

/// Multi-tier self-healing working directory resolution.
/// Ensures the returned PathBuf is an existing, usable directory on disk.
pub fn resolve_working_dir(desired: &str) -> PathBuf {
    let trimmed = desired.trim();
    if !trimmed.is_empty() {
        let path = PathBuf::from(trimmed);
        if path.is_dir() {
            return path;
        }
        if std::fs::create_dir_all(&path).is_ok() && path.is_dir() {
            return path;
        }
    }

    // Tier 2: Dedicated unique temp dir
    let temp_unique = std::env::temp_dir().join(format!("tuzygent_workdir_{}", uuid::Uuid::new_v4()));
    if std::fs::create_dir_all(&temp_unique).is_ok() && temp_unique.is_dir() {
        return temp_unique;
    }

    // Tier 3: target/tmp
    let target_tmp = PathBuf::from("target/tmp");
    if std::fs::create_dir_all(&target_tmp).is_ok() && target_tmp.is_dir() {
        return target_tmp;
    }

    // Tier 4: System /tmp
    let sys_tmp = PathBuf::from("/tmp");
    if sys_tmp.is_dir() {
        return sys_tmp;
    }

    // Final fallback: current executable dir or "."
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

pub fn command(
    input: &EngineInput,
    binary: &str,
    args: &[String],
    env: &HashMap<String, String>,
) -> Command {
    let mut cmd = if let Some(sandbox) = &input.command_sandbox {
        let mut cmd = Command::new(&sandbox.program);
        let mut prefix = sandbox.prefix_args.clone();
        for (key, value) in env {
            if sandbox.env_whitelist.contains(key) {
                let extra = if sandbox.program == "bwrap" {
                    vec!["--setenv".into(), key.clone(), value.clone()]
                } else {
                    vec!["--env".into(), format!("{key}={value}")]
                };
                if sandbox.program == "docker" {
                    let image_position = prefix.len().saturating_sub(1);
                    prefix.splice(image_position..image_position, extra);
                } else {
                    prefix.extend(extra);
                }
            }
        }
        cmd.args(&prefix);
        // Allow explicitly selected standalone executables outside the system runtime mounts.
        if sandbox.program == "bwrap" && Path::new(binary).is_absolute() {
            if let Ok(source) = std::fs::canonicalize(binary) {
                cmd.arg("--ro-bind").arg(source).arg(binary);
            }
        }
        if sandbox.program == "bwrap" {
            cmd.arg("--");
        }
        cmd.arg(binary).args(args);
        cmd.env_clear();
        for key in sandbox
            .env_whitelist
            .iter()
            .map(String::as_str)
            .chain(["PATH", "LANG", "HOME"])
        {
            if let Ok(value) = std::env::var(key) {
                cmd.env(key, value);
            }
        }
        for (key, value) in env {
            if sandbox.env_whitelist.contains(key) {
                cmd.env(key, value);
            }
        }
        cmd
    } else {
        let mut cmd = Command::new(binary);
        cmd.args(args).envs(env);
        cmd
    };

    let resolved_dir = resolve_working_dir(&input.working_dir);
    cmd.current_dir(resolved_dir);
    cmd
}

pub fn timeout(input: &EngineInput) -> Duration {
    Duration::from_secs(if input.timeout_seconds == 0 {
        300
    } else {
        input.timeout_seconds
    })
}

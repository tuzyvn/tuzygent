use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::command::wrap_command;
use super::config::SandboxConfig;
use super::driver::SandboxDriverKind;

/// Minimal sysfs CPU topology files exposed read-only so TCMalloc/Antigravity runtimes do not SIGABRT.
pub const SANDBOX_CPU_TOPOLOGY_PATHS: &[&str] = &[
    "/sys/devices/system/cpu/possible",
    "/sys/devices/system/cpu/online",
];

/// Control-plane and credential env names that must never cross into a sandboxed child even if listed.
pub const SANDBOX_ENV_NEVER_LIST: &[&str] = &[
    "OMB_LICENSE_KEY",
    "OMB_INSTALLATION_CREDENTIAL",
    "OMB_LOOPBACK_TRUST",
    "OMB_CLOUD_SECRETS_FD",
    "OMB_DATA_DIR",
    "OMB_STATIC_DIR",
    "OMB_DUCKDB_DIR",
    "OMB_PORT",
    "OMB_WEBHOOK_PORT",
    "OPENMAUSBOT_TOKEN",
    "OPENMAUSBOT_URL",
    "TUZYGENT_MASTER_KEY",
    "TUZYGENT_AUTH_TOKEN",
    "TUZYGENT_ADMIN_TOKEN",
    "TUZYGENT_DB_PATH",
    "TUZYGENT_LICENSE_KEY",
    "TUZYGENT_SECRETS_FD",
    "TUZYGENT_LOOPBACK_TRUST",
    "AGENT_SERVER_SECRET",
    "AGENT_CONTROL_TOKEN",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "GITHUB_TOKEN",
    "GH_TOKEN",
    "OPENAI_ADMIN_KEY",
    "ANTHROPIC_ADMIN_KEY",
];

const SANDBOX_ENV_NEVER_PREFIXES: &[&str] = &["TUZYGENT_INTERNAL_", "OMB_CLOUD_"];

fn is_never_env_key(key: &str) -> bool {
    let upper = key.trim().to_ascii_uppercase();
    upper.is_empty()
        || SANDBOX_ENV_NEVER_LIST.contains(&upper.as_str())
        || SANDBOX_ENV_NEVER_PREFIXES
            .iter()
            .any(|prefix| upper.starts_with(prefix))
}

/// Return true if `key` appears in `whitelist` and is not forbidden by the control-plane never-list.
pub fn is_sandbox_env_allowed(key: &str, whitelist: &[String]) -> bool {
    if is_never_env_key(key) {
        return false;
    }
    whitelist.iter().any(|allowed| allowed == key)
}

/// Filter a configured environment whitelist through the sandbox never-list while keeping order.
pub fn filter_sandbox_env_whitelist(whitelist: &[String]) -> Vec<String> {
    let mut seen = HashSet::with_capacity(whitelist.len());
    let mut out = Vec::with_capacity(whitelist.len());
    for key in whitelist {
        if is_sandbox_env_allowed(key, whitelist) && seen.insert(key.clone()) {
            out.push(key.clone());
        }
    }
    out
}

/// Deliberate weakening mutations used by tests/probes to prove self-check detects real isolation holes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxMutation {
    BindRoot,
    SharePid,
    ExposeNeverEnv,
    SharedTmp,
}

/// Structural verification report over a generated sandbox command plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxSelfCheckReport {
    pub driver: String,
    pub passed: bool,
    pub mutation: Option<SandboxMutation>,
    pub checks: Vec<String>,
    pub failures: Vec<String>,
}

fn driver_label(driver: &SandboxDriverKind) -> &'static str {
    match driver {
        SandboxDriverKind::Bubblewrap => "bubblewrap",
        SandboxDriverKind::Docker => "docker",
        SandboxDriverKind::Host => "host",
    }
}

/// Verify structural isolation properties of `wrap_command` with an optional synthetic regression `mutation`.
pub fn verify_sandbox_plan(
    config: &SandboxConfig,
    run_dir: &Path,
    mutation: Option<SandboxMutation>,
) -> SandboxSelfCheckReport {
    let (_, mut args) = wrap_command(config, "probe-cmd", &["--version".to_string()], run_dir);
    let mut checks = Vec::new();
    let mut failures = Vec::new();

    match config.driver {
        SandboxDriverKind::Bubblewrap => {
            apply_bwrap_mutation(&mut args, mutation);
            verify_bwrap_plan(&args, &mut checks, &mut failures);
        }
        SandboxDriverKind::Docker => {
            apply_docker_mutation(&mut args, mutation);
            verify_docker_plan(&args, &mut checks, &mut failures);
        }
        SandboxDriverKind::Host => {
            checks.push("host_driver_checked".to_string());
            failures.push("host_driver_has_no_isolation".to_string());
        }
    }

    SandboxSelfCheckReport {
        driver: driver_label(&config.driver).to_string(),
        passed: failures.is_empty(),
        mutation,
        checks,
        failures,
    }
}

fn apply_bwrap_mutation(args: &mut Vec<String>, mutation: Option<SandboxMutation>) {
    let Some(kind) = mutation else {
        return;
    };
    let insert_at = args.len().saturating_sub(2);
    match kind {
        SandboxMutation::BindRoot => {
            args.splice(insert_at..insert_at, ["--ro-bind", "/", "/"].map(String::from));
        }
        SandboxMutation::SharePid => {
            args.retain(|a| a != "--unshare-pid");
        }
        SandboxMutation::ExposeNeverEnv => {
            args.splice(
                insert_at..insert_at,
                ["--setenv", "TUZYGENT_MASTER_KEY", "mutated"].map(String::from),
            );
        }
        SandboxMutation::SharedTmp => {
            let mut filtered = Vec::with_capacity(args.len());
            let mut i = 0;
            while i < args.len() {
                if args.get(i).map(String::as_str) == Some("--tmpfs")
                    && args.get(i + 1).map(String::as_str) == Some("/tmp")
                {
                    i += 2;
                    continue;
                }
                if let Some(item) = args.get(i) {
                    filtered.push(item.clone());
                }
                i += 1;
            }
            *args = filtered;
        }
    }
}

fn apply_docker_mutation(args: &mut Vec<String>, mutation: Option<SandboxMutation>) {
    let Some(kind) = mutation else {
        return;
    };
    let insert_at = args.len().saturating_sub(3);
    match kind {
        SandboxMutation::BindRoot => {
            args.splice(insert_at..insert_at, ["-v", "/:/host"].map(String::from));
        }
        SandboxMutation::SharePid => {
            args.retain(|a| !a.starts_with("--pids-limit="));
            args.splice(insert_at..insert_at, ["--pid=host".to_string()]);
        }
        SandboxMutation::ExposeNeverEnv => {
            args.splice(
                insert_at..insert_at,
                ["-e", "TUZYGENT_MASTER_KEY=mutated"].map(String::from),
            );
        }
        SandboxMutation::SharedTmp => {
            args.splice(insert_at..insert_at, ["-v", "/tmp:/tmp"].map(String::from));
        }
    }
}

fn verify_bwrap_plan(args: &[String], checks: &mut Vec<String>, failures: &mut Vec<String>) {
    checks.push("no_root_bind".to_string());
    if args
        .windows(3)
        .any(|w| w == ["--ro-bind", "/", "/"] || w == ["--bind", "/", "/"])
    {
        failures.push("root_filesystem_exposed".to_string());
    }

    checks.push("pid_isolated".to_string());
    if !args.iter().any(|a| a == "--unshare-pid" || a == "--unshare-all") {
        failures.push("missing_pid_namespace".to_string());
    }

    checks.push("private_tmp".to_string());
    if !args.windows(2).any(|w| w == ["--tmpfs", "/tmp"]) {
        failures.push("missing_private_tmpfs".to_string());
    }

    checks.push("never_env_blocked".to_string());
    if args
        .windows(3)
        .any(|w| w[0] == "--setenv" && is_never_env_key(&w[1]))
    {
        failures.push("never_list_env_exposed".to_string());
    }

    checks.push("proc_and_dev_isolated".to_string());
    if !args.windows(2).any(|w| w == ["--proc", "/proc"])
        || !args.windows(2).any(|w| w == ["--dev", "/dev"])
        || !args.iter().any(|a| a == "--die-with-parent")
    {
        failures.push("missing_proc_dev_or_die_with_parent".to_string());
    }

    checks.push("cpu_topology_ro_bound".to_string());
    for cpu_path in SANDBOX_CPU_TOPOLOGY_PATHS {
        if Path::new(cpu_path).exists()
            && !args
                .windows(3)
                .any(|w| w == ["--ro-bind", *cpu_path, *cpu_path])
        {
            failures.push(format!("missing_cpu_topology_bind:{cpu_path}"));
        }
    }
}

fn verify_docker_plan(args: &[String], checks: &mut Vec<String>, failures: &mut Vec<String>) {
    checks.push("no_root_bind".to_string());
    if args
        .windows(2)
        .any(|w| w[0] == "-v" && (w[1].starts_with("/:/") || w[1] == "/:/host"))
    {
        failures.push("root_filesystem_exposed".to_string());
    }

    checks.push("pid_isolated".to_string());
    if args.iter().any(|a| a == "--pid=host") || !args.iter().any(|a| a.starts_with("--pids-limit=")) {
        failures.push("missing_pid_isolation".to_string());
    }

    checks.push("private_tmp".to_string());
    if args.windows(2).any(|w| w == ["-v", "/tmp:/tmp"]) {
        failures.push("host_tmp_shared".to_string());
    }

    checks.push("never_env_blocked".to_string());
    for w in args.windows(2) {
        if w[0] == "-e" {
            let key = w[1].split('=').next().unwrap_or_default();
            if is_never_env_key(key) {
                failures.push("never_list_env_exposed".to_string());
            }
        }
    }

    checks.push("capabilities_dropped".to_string());
    if !args.iter().any(|a| a == "--cap-drop=ALL")
        || !args.iter().any(|a| a == "--security-opt=no-new-privileges")
    {
        failures.push("missing_docker_capability_drop".to_string());
    }
}

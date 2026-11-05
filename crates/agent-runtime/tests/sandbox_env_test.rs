//! Comprehensive integration test suite for SandboxEnvironment in agent-runtime.
//! Tests all isolation modes (LocalDirectory, Ephemeral, Git Worktree),
//! command wrapping across drivers (Host, Bubblewrap, Docker),
//! and context injection & purge behaviors.

use agent_engine::config::CredentialMount;
use agent_runtime::{
    expand_home, IsolationMode, SandboxConfig, SandboxDriverKind, SandboxEnvironment,
};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Helper to create a unique temporary directory for tests.
fn unique_test_dir(prefix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("{}_{}", prefix, Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).expect("Failed to create temporary test directory");
    dir
}

/// Helper to initialize a real git repository with an initial commit for worktree testing.
async fn create_test_git_repo() -> PathBuf {
    let repo_dir = unique_test_dir("sandbox_git_repo");

    let run_git = |args: &[&str]| {
        let repo = repo_dir.clone();
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        async move {
            let status = tokio::process::Command::new("git")
                .current_dir(&repo)
                .args(&args)
                .status()
                .await
                .expect("Failed to execute git process");
            assert!(status.success(), "git command failed: {:?}", args);
        }
    };

    run_git(&["init"]).await;
    run_git(&["config", "user.name", "Agent Runtime Tester"]).await;
    run_git(&["config", "user.email", "agent-tester@enterprise.local"]).await;
    tokio::fs::write(repo_dir.join("README.md"), "# Sandbox Test Repo\n")
        .await
        .expect("Failed to write initial README");
    run_git(&["add", "README.md"]).await;
    run_git(&["commit", "-m", "Initial test commit"]).await;

    repo_dir
}

// ============================================================================
// 1. ISOLATION MODES (LocalDirectory, Ephemeral, Git Worktree)
// ============================================================================

#[tokio::test]
async fn test_isolation_mode_local_directory() {
    let temp_dir = unique_test_dir("local_dir");
    let mut env = SandboxEnvironment::new(&temp_dir, IsolationMode::LocalDirectory);

    assert_eq!(env.config.mode, IsolationMode::LocalDirectory);
    assert!(env.worktree_path.is_none());
    assert!(env.branch_name.is_none());

    let prepared_path = env.prepare("task-local-1").await.expect("prepare failed");
    assert_eq!(prepared_path, temp_dir);
    assert!(env.worktree_path.is_none());
    assert!(env.branch_name.is_none());

    // setup_workspace delegates directly to prepare
    let setup_path = env
        .setup_workspace("task-local-1")
        .await
        .expect("setup_workspace failed");
    assert_eq!(setup_path, temp_dir);

    // teardown should succeed cleanly without modifying worktree_path
    let teardown_res = env.teardown().await;
    assert!(teardown_res.is_ok());
    assert!(env.worktree_path.is_none());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_isolation_mode_ephemeral_lifecycle() {
    let base_dir = unique_test_dir("ephemeral_base");
    let mut env = SandboxEnvironment::new(&base_dir, IsolationMode::Ephemeral);

    let prepared_path = env.prepare("task_eph_99").await.expect("prepare failed");
    assert!(prepared_path.exists());
    assert!(prepared_path.is_dir());
    assert!(prepared_path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("agent-sandbox-"));

    assert_eq!(env.worktree_path, Some(prepared_path.clone()));
    assert!(env.branch_name.is_none());

    // Create a dummy file inside ephemeral directory to verify write operations
    let sample_file = prepared_path.join("ephemeral_artifact.txt");
    tokio::fs::write(&sample_file, "temporary data")
        .await
        .expect("Failed to write to ephemeral sandbox");
    assert!(sample_file.exists());

    // Teardown should remove the ephemeral directory and reset worktree_path
    env.teardown().await.expect("teardown failed");
    assert!(!prepared_path.exists());
    assert!(env.worktree_path.is_none());

    // Multiple teardowns are idempotent and safe
    assert!(env.teardown().await.is_ok());

    let _ = std::fs::remove_dir_all(&base_dir);
}

#[tokio::test]
async fn test_isolation_mode_git_worktree_lifecycle() {
    let repo_dir = create_test_git_repo().await;
    let mut env = SandboxEnvironment::new(&repo_dir, IsolationMode::Worktree);

    let task_id = "TASK_FEATURE_A";
    let worktree_dir = env.prepare(task_id).await.expect("prepare worktree failed");

    // Branch name should normalize uppercase and underscores
    let expected_branch = "agent/task-task-feature-a";
    assert_eq!(env.branch_name.as_deref(), Some(expected_branch));
    assert_eq!(env.worktree_path.as_ref(), Some(&worktree_dir));

    // The worktree directory must exist and contain git repo files
    assert!(worktree_dir.exists());
    assert!(worktree_dir.join("README.md").exists());

    // Verify git branch exists
    let branch_check = tokio::process::Command::new("git")
        .current_dir(&repo_dir)
        .args(["branch", "--list", expected_branch])
        .output()
        .await
        .expect("Failed to list git branches");
    let branch_output = String::from_utf8_lossy(&branch_check.stdout);
    assert!(
        branch_output.contains(expected_branch),
        "Branch {} was not found in git branch list",
        expected_branch
    );

    // Teardown should remove the worktree
    env.teardown().await.expect("teardown failed");
    assert!(env.worktree_path.is_none());
    assert!(!worktree_dir.exists());

    // Clean up repo
    let _ = tokio::fs::remove_dir_all(&repo_dir).await;
}

#[tokio::test]
async fn test_isolation_mode_git_worktree_fails_on_non_git_repo() {
    let non_git_dir = unique_test_dir("non_git_repo");
    let mut env = SandboxEnvironment::new(&non_git_dir, IsolationMode::Worktree);

    let res = env.prepare("task-invalid").await;
    assert!(res.is_err(), "Worktree should fail when repo is not a valid git repository");

    let _ = std::fs::remove_dir_all(&non_git_dir);
}

// ============================================================================
// 2. DRIVER & COMMAND WRAPPING (Host, Bubblewrap, Docker)
// ============================================================================

#[test]
fn test_wrap_command_host_driver() {
    let config = SandboxConfig {
        driver: SandboxDriverKind::Host,
        ..Default::default()
    };
    let env = SandboxEnvironment::with_config(".", config);

    let args = vec!["-l".to_string(), "/tmp".to_string()];
    let (bin, wrapped_args) = env.wrap_command("ls", &args, Path::new("/tmp"));

    assert_eq!(bin, "ls");
    assert_eq!(wrapped_args, args);

    let sandbox_spec = env.command_sandbox(Path::new("/tmp"));
    assert!(sandbox_spec.is_none(), "Host driver must not return a CommandSandbox");
}

#[test]
fn test_wrap_command_bubblewrap_isolation_flags() {
    let config = SandboxConfig {
        driver: SandboxDriverKind::Bubblewrap,
        ..Default::default()
    };
    let env = SandboxEnvironment::with_config(".", config);

    let target_dir = std::env::temp_dir();
    let (bin, args) = env.wrap_command("cargo", &["test".to_string()], &target_dir);

    assert_eq!(bin, "bwrap");

    // Standard unshare & isolation flags
    let required_flags = [
        "--unshare-pid",
        "--unshare-ipc",
        "--unshare-uts",
        "--die-with-parent",
        "--new-session",
        "--dev",
        "/dev",
        "--proc",
        "/proc",
        "--tmpfs",
        "/tmp",
    ];
    for flag in required_flags {
        assert!(
            args.iter().any(|a| a == flag),
            "Bubblewrap missing flag: {}",
            flag
        );
    }

    // HOME and /root setting
    assert!(args.windows(2).any(|w| w == ["--dir", "/root"]));
    assert!(args.windows(3).any(|w| w == ["--setenv", "HOME", "/root"]));

    // Working directory bind and chdir
    let canonical_target = std::fs::canonicalize(&target_dir).unwrap_or(target_dir);
    let target_str = canonical_target.to_string_lossy().to_string();
    assert!(args
        .windows(3)
        .any(|w| w == ["--bind", target_str.as_str(), target_str.as_str()]));
    assert!(args
        .windows(2)
        .any(|w| w == ["--chdir", target_str.as_str()]));

    // Root filesystem must NOT be mapped as root
    assert!(!args.windows(3).any(|w| w == ["--ro-bind", "/", "/"]));

    // Original command and arguments must be trailing
    assert_eq!(args.iter().rev().take(2).collect::<Vec<_>>(), vec![
        &"test".to_string(),
        &"cargo".to_string(),
    ]);
}

#[test]
fn test_wrap_command_bubblewrap_binary_parent_mount() {
    let custom_bin_dir = unique_test_dir("custom_bin_dir");
    let custom_bin = custom_bin_dir.join("my_compiler");
    std::fs::write(&custom_bin, "#!/bin/sh\necho ok").unwrap();

    let config = SandboxConfig {
        driver: SandboxDriverKind::Bubblewrap,
        ..Default::default()
    };
    let env = SandboxEnvironment::with_config(".", config);

    let bin_path_str = custom_bin.to_string_lossy().to_string();
    let (_, args) = env.wrap_command(&bin_path_str, &[], Path::new("/tmp"));

    let parent_str = custom_bin_dir.to_string_lossy().to_string();
    assert!(
        args.windows(3)
            .any(|w| w == ["--ro-bind", parent_str.as_str(), parent_str.as_str()]),
        "Bubblewrap should ro-bind external binary directory outside system folders"
    );

    let _ = std::fs::remove_dir_all(&custom_bin_dir);
}

#[test]
fn test_wrap_command_bubblewrap_credential_and_env_injection() {
    let cred_dir = unique_test_dir("sandbox_creds");
    let cred_file = cred_dir.join("test_token.json");
    std::fs::write(&cred_file, "secret-token").unwrap();

    let non_existent_cred = cred_dir.join("missing.json");

    let test_env_key = "TU_TEST_BWRAP_VAR";
    let test_env_val = "authenticated_value_123";
    std::env::set_var(test_env_key, test_env_val);

    let missing_env_key = "TU_TEST_BWRAP_UNSET";
    std::env::remove_var(missing_env_key);

    let config = SandboxConfig {
        driver: SandboxDriverKind::Bubblewrap,
        ro_credentials: vec![
            CredentialMount {
                source: cred_file.to_string_lossy().to_string(),
                target: "/root/token.json".to_string(),
                readonly: true,
            },
            CredentialMount {
                source: non_existent_cred.to_string_lossy().to_string(),
                target: "/root/missing.json".to_string(),
                readonly: true,
            },
        ],
        env_whitelist: vec![test_env_key.to_string(), missing_env_key.to_string()],
        ..Default::default()
    };
    let env = SandboxEnvironment::with_config(".", config);

    let (_, args) = env.wrap_command("whoami", &[], Path::new("/tmp"));

    // Existing credential mounted
    assert!(args.windows(3).any(|w| w
        == [
            "--ro-bind",
            cred_file.to_string_lossy().as_ref(),
            "/root/token.json"
        ]));
    // Non-existent credential skipped
    assert!(!args.iter().any(|a| a == "/root/missing.json"));

    // Set environment variable injected
    assert!(args
        .windows(3)
        .any(|w| w == ["--setenv", test_env_key, test_env_val]));
    // Unset environment variable NOT injected
    assert!(!args
        .windows(2)
        .any(|w| w == ["--setenv", missing_env_key]));

    std::env::remove_var(test_env_key);
    let _ = std::fs::remove_dir_all(&cred_dir);
}

#[test]
fn test_wrap_command_docker_driver() {
    let cred_dir = unique_test_dir("docker_creds");
    let cred_file = cred_dir.join("docker_key.pem");
    std::fs::write(&cred_file, "pem-data").unwrap();

    let test_env_key = "TU_TEST_DOCKER_VAR";
    let test_env_val = "docker_secret";
    std::env::set_var(test_env_key, test_env_val);

    let config = SandboxConfig {
        driver: SandboxDriverKind::Docker,
        docker_image: "my-custom-engine:v1".to_string(),
        memory_limit_mb: Some(2048),
        ro_credentials: vec![CredentialMount {
            source: cred_file.to_string_lossy().to_string(),
            target: "/root/docker_key.pem".to_string(),
            readonly: true,
        }],
        env_whitelist: vec![test_env_key.to_string()],
        ..Default::default()
    };
    let env = SandboxEnvironment::with_config(".", config);

    let target_dir = std::env::temp_dir();
    let (bin, args) = env.wrap_command("python", &["-c".to_string(), "print(1)".to_string()], &target_dir);

    assert_eq!(bin, "docker");
    assert_eq!(args[0], "run");
    assert!(args.contains(&"--rm".to_string()));
    assert!(args.contains(&"-i".to_string()));
    assert!(args.contains(&"--cap-drop=ALL".to_string()));
    assert!(args.contains(&"--security-opt=no-new-privileges".to_string()));
    assert!(args.contains(&"--pids-limit=256".to_string()));

    // Memory limit
    assert!(args.windows(2).any(|w| w == ["--memory", "2048m"]));

    // Workspace volume & working directory
    let canonical_target = std::fs::canonicalize(&target_dir).unwrap_or(target_dir);
    let vol_mapping = format!("{}:/workspace", canonical_target.to_string_lossy());
    assert!(args.windows(2).any(|w| w == ["-v", vol_mapping.as_str()]));
    assert!(args.windows(2).any(|w| w == ["-w", "/workspace"]));

    // Credential volume mount
    let cred_mapping = format!(
        "{}:/root/docker_key.pem:ro",
        cred_file.to_string_lossy()
    );
    assert!(args.windows(2).any(|w| w == ["-v", cred_mapping.as_str()]));

    // Environment injection
    let env_expected = format!("{}={}", test_env_key, test_env_val);
    assert!(args.windows(2).any(|w| w == ["-e", env_expected.as_str()]));

    // Image and command trailing
    let last_elements = args.iter().rev().take(4).cloned().collect::<Vec<_>>();
    assert_eq!(
        last_elements,
        vec![
            "print(1)".to_string(),
            "-c".to_string(),
            "python".to_string(),
            "my-custom-engine:v1".to_string()
        ]
    );

    std::env::remove_var(test_env_key);
    let _ = std::fs::remove_dir_all(&cred_dir);
}

#[test]
fn test_wrap_command_docker_driver_without_memory_limit() {
    let config = SandboxConfig {
        driver: SandboxDriverKind::Docker,
        memory_limit_mb: None,
        ..Default::default()
    };
    let env = SandboxEnvironment::with_config(".", config);

    let (bin, args) = env.wrap_command("true", &[], Path::new("/tmp"));
    assert_eq!(bin, "docker");
    assert!(!args.contains(&"--memory".to_string()));
}

#[test]
fn test_command_sandbox_spec_generation() {
    let run_dir = std::env::temp_dir();

    // Bubblewrap
    let bwrap_config = SandboxConfig {
        driver: SandboxDriverKind::Bubblewrap,
        env_whitelist: vec!["MY_VAR".to_string()],
        ..Default::default()
    };
    let bwrap_env = SandboxEnvironment::with_config(".", bwrap_config);
    let bwrap_sandbox = bwrap_env
        .command_sandbox(&run_dir)
        .expect("bwrap command_sandbox should return Some");
    assert_eq!(bwrap_sandbox.program, "bwrap");
    assert!(bwrap_sandbox.container_name.is_none());
    assert!(!bwrap_sandbox.prefix_args.contains(&"__engine__".to_string()));
    assert_eq!(bwrap_sandbox.env_whitelist, vec!["MY_VAR".to_string()]);

    // Docker
    let docker_config = SandboxConfig {
        driver: SandboxDriverKind::Docker,
        ..Default::default()
    };
    let docker_env = SandboxEnvironment::with_config(".", docker_config);
    let docker_sandbox = docker_env
        .command_sandbox(&run_dir)
        .expect("docker command_sandbox should return Some");
    assert_eq!(docker_sandbox.program, "docker");
    assert!(docker_sandbox.container_name.is_some());
    let cname = docker_sandbox.container_name.unwrap();
    assert!(cname.starts_with("tuzygent-"));
    assert_eq!(docker_sandbox.working_dir, "/workspace");
    assert_eq!(docker_sandbox.prefix_args[0], "run");
    assert_eq!(docker_sandbox.prefix_args[1], "--name");
    assert_eq!(docker_sandbox.prefix_args[2], cname);
    assert!(!docker_sandbox.prefix_args.contains(&"__engine__".to_string()));
}

// ============================================================================
// 3. CONTEXT INJECTION & PURGE
// ============================================================================

#[tokio::test]
async fn test_context_injection_and_purge() {
    let workspace = unique_test_dir("context_test_ws");
    let env = SandboxEnvironment::new(&workspace, IsolationMode::LocalDirectory);

    let brief = "# Task Briefing\nExecute feature XYZ with utmost care.";
    env.inject_agent_brief(&workspace, brief)
        .await
        .expect("Failed to inject agent brief");

    let multica_dir = workspace.join(".multica");
    let brief_file = multica_dir.join("brief.md");
    let agent_ctx = workspace.join(".agent_context.md");

    assert!(multica_dir.is_dir());
    assert!(brief_file.is_file());
    assert!(agent_ctx.is_file());

    let brief_read = tokio::fs::read_to_string(&brief_file).await.unwrap();
    assert_eq!(brief_read, brief);

    let agent_ctx_read = tokio::fs::read_to_string(&agent_ctx).await.unwrap();
    assert_eq!(agent_ctx_read, brief);

    // Add execution_context.md and a persistent sidecar configuration file
    let exec_file = multica_dir.join("execution_context.md");
    tokio::fs::write(&exec_file, "runtime state").await.unwrap();

    let persistent_file = multica_dir.join("persistent_config.json");
    tokio::fs::write(&persistent_file, "{\"preserve\": true}").await.unwrap();

    // Purge sidecar context
    env.purge_sidecar_context(&workspace)
        .await
        .expect("purge_sidecar_context failed");

    // ephemeral briefing & context files must be deleted
    assert!(!agent_ctx.exists(), ".agent_context.md should be deleted");
    assert!(!brief_file.exists(), ".multica/brief.md should be deleted");
    assert!(!exec_file.exists(), ".multica/execution_context.md should be deleted");

    // Persistent files and the .multica directory must remain intact
    assert!(multica_dir.exists(), ".multica directory must NOT be deleted");
    assert!(persistent_file.exists(), "Persistent files inside .multica must be preserved");

    let persistent_read = tokio::fs::read_to_string(&persistent_file).await.unwrap();
    assert_eq!(persistent_read, "{\"preserve\": true}");

    // Purge should be idempotent
    assert!(env.purge_sidecar_context(&workspace).await.is_ok());

    let _ = tokio::fs::remove_dir_all(&workspace).await;
}

// ============================================================================
// 4. UTILITIES & CONFIGURATION
// ============================================================================

#[test]
fn test_expand_home() {
    let original_home = std::env::var("HOME").ok();
    let mock_home = "/home/testuser";
    std::env::set_var("HOME", mock_home);

    assert_eq!(
        expand_home("~/config/key.pem"),
        PathBuf::from("/home/testuser/config/key.pem")
    );
    assert_eq!(expand_home("~"), PathBuf::from("/home/testuser"));
    assert_eq!(
        expand_home("/etc/ssl/certs"),
        PathBuf::from("/etc/ssl/certs")
    );
    assert_eq!(
        expand_home("relative/path/test"),
        PathBuf::from("relative/path/test")
    );

    if let Some(h) = original_home {
        std::env::set_var("HOME", h);
    }
}

#[test]
fn test_driver_kind_parse_and_display() {
    assert_eq!(
        SandboxDriverKind::parse("docker").unwrap(),
        SandboxDriverKind::Docker
    );
    assert_eq!(
        SandboxDriverKind::parse("DOCKER").unwrap(),
        SandboxDriverKind::Docker
    );
    assert_eq!(
        SandboxDriverKind::parse("bubblewrap").unwrap(),
        SandboxDriverKind::Bubblewrap
    );
    assert_eq!(
        SandboxDriverKind::parse("bwrap").unwrap(),
        SandboxDriverKind::Bubblewrap
    );
    assert_eq!(
        SandboxDriverKind::parse("BWRAP").unwrap(),
        SandboxDriverKind::Bubblewrap
    );
    assert_eq!(
        SandboxDriverKind::parse("host").unwrap(),
        SandboxDriverKind::Host
    );
    assert_eq!(
        SandboxDriverKind::parse("HOST").unwrap(),
        SandboxDriverKind::Host
    );

    assert!(SandboxDriverKind::parse("unknown_driver").is_err());

    assert_eq!(SandboxDriverKind::Docker.to_string(), "docker");
    assert_eq!(SandboxDriverKind::Bubblewrap.to_string(), "bubblewrap");
    assert_eq!(SandboxDriverKind::Host.to_string(), "host");
}

#[test]
fn test_detect_best_driver() {
    let driver = SandboxConfig::detect_best_driver();
    assert!(matches!(
        driver,
        SandboxDriverKind::Bubblewrap | SandboxDriverKind::Docker | SandboxDriverKind::Host
    ));
}

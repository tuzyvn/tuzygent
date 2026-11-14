use std::path::Path;

use agent_runtime::{
    filter_sandbox_env_whitelist, is_sandbox_env_allowed, verify_sandbox_plan, wrap_command,
    SandboxConfig, SandboxDriverKind, SandboxMutation, SandboxSelfCheckReport,
    SANDBOX_CPU_TOPOLOGY_PATHS, SANDBOX_ENV_NEVER_LIST,
};

#[test]
fn sandbox_env_never_list_blocks_control_plane_even_if_whitelisted() {
    assert!(!SANDBOX_CPU_TOPOLOGY_PATHS.is_empty());
    assert!(SANDBOX_ENV_NEVER_LIST.contains(&"TUZYGENT_MASTER_KEY"));
    assert!(SANDBOX_ENV_NEVER_LIST.contains(&"OMB_LICENSE_KEY"));

    let whitelist = vec![
        "SAFE_CUSTOM_FLAG".to_string(),
        "TUZYGENT_MASTER_KEY".to_string(),
        "TUZYGENT_INTERNAL_SECRET".to_string(),
        "OMB_CLOUD_API_KEY".to_string(),
        "OMB_PORT".to_string(),
        "SAFE_CUSTOM_FLAG".to_string(),
    ];

    assert!(is_sandbox_env_allowed("SAFE_CUSTOM_FLAG", &whitelist));
    assert!(!is_sandbox_env_allowed("TUZYGENT_MASTER_KEY", &whitelist));
    assert!(!is_sandbox_env_allowed("TUZYGENT_INTERNAL_SECRET", &whitelist));
    assert!(!is_sandbox_env_allowed("OMB_CLOUD_API_KEY", &whitelist));
    assert!(!is_sandbox_env_allowed("OMB_PORT", &whitelist));
    assert!(!is_sandbox_env_allowed("UNLISTED_VAR", &whitelist));

    let filtered = filter_sandbox_env_whitelist(&whitelist);
    assert_eq!(filtered, vec!["SAFE_CUSTOM_FLAG".to_string()]);
}

#[test]
fn bwrap_mounts_cpu_topology_paths_and_drops_never_list_env() {
    let key_safe = "TU_SELF_CHECK_SAFE";
    let key_never = "TUZYGENT_MASTER_KEY";
    std::env::set_var(key_safe, "ok-val");
    std::env::set_var(key_never, "must-never-leak");

    let config = SandboxConfig {
        driver: SandboxDriverKind::Bubblewrap,
        env_whitelist: vec![key_safe.to_string(), key_never.to_string()],
        ..Default::default()
    };
    let (_, args) = wrap_command(&config, "echo", &["hi".to_string()], Path::new("/tmp"));

    for cpu_path in SANDBOX_CPU_TOPOLOGY_PATHS {
        if Path::new(cpu_path).exists() {
            assert!(
                args.windows(3).any(|w| w == ["--ro-bind", *cpu_path, *cpu_path]),
                "missing ro-bind for {cpu_path}"
            );
        }
    }
    assert!(args
        .windows(3)
        .any(|w| w == ["--setenv", key_safe, "ok-val"]));
    assert!(!args.windows(2).any(|w| w == ["--setenv", key_never]));

    std::env::remove_var(key_safe);
    std::env::remove_var(key_never);
}

#[test]
fn verify_sandbox_plan_passes_cleanly_and_detects_every_mutation() {
    let bwrap = SandboxConfig {
        driver: SandboxDriverKind::Bubblewrap,
        ..Default::default()
    };
    let clean = verify_sandbox_plan(&bwrap, Path::new("/tmp"), None);
    assert!(clean.passed, "expected clean bwrap plan, got {:?}", clean.failures);
    assert_eq!(clean.driver, "bubblewrap");

    let json = serde_json::to_string(&clean).unwrap();
    let decoded: SandboxSelfCheckReport = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, clean);

    for mutation in [
        SandboxMutation::BindRoot,
        SandboxMutation::SharePid,
        SandboxMutation::ExposeNeverEnv,
        SandboxMutation::SharedTmp,
    ] {
        let mut_json = serde_json::to_string(&mutation).unwrap();
        let mut_decoded: SandboxMutation = serde_json::from_str(&mut_json).unwrap();
        assert_eq!(mut_decoded, mutation);

        let report = verify_sandbox_plan(&bwrap, Path::new("/tmp"), Some(mutation));
        assert!(
            !report.passed && !report.failures.is_empty(),
            "mutation {mutation:?} should fail bwrap self-check"
        );
    }

    let docker = SandboxConfig {
        driver: SandboxDriverKind::Docker,
        ..Default::default()
    };
    assert!(verify_sandbox_plan(&docker, Path::new("/tmp"), None).passed);
    assert!(
        !verify_sandbox_plan(&docker, Path::new("/tmp"), Some(SandboxMutation::SharePid)).passed
    );

    let host = SandboxConfig {
        driver: SandboxDriverKind::Host,
        ..Default::default()
    };
    assert!(!verify_sandbox_plan(&host, Path::new("/tmp"), None).passed);
}

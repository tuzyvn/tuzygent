#![cfg(test)]

use agent_domain::VerificationSpec;
use std::path::Path;

use super::TaskVerifier;

#[test]
fn test_parse_specs_recognizes_supported_verification_descriptions() {
    let temp_dir = std::env::temp_dir();
    let verifier = TaskVerifier::new(&temp_dir);
    let specs = verifier.parse_specs(
            "- command: cargo check\n* file_exists: report.txt\n• json: data.json\ncontains: report.txt | finished\npytest tests/\nReview deployment manually",
        );

    assert_eq!(
        specs,
        vec![
            VerificationSpec::Command {
                command: "cargo check".into(),
                expected_exit_code: 0,
            },
            VerificationSpec::FileExists {
                path: "report.txt".into(),
                min_bytes: Some(1),
            },
            VerificationSpec::JsonValid {
                path: "data.json".into(),
            },
            VerificationSpec::ContentContains {
                path: "report.txt".into(),
                pattern: "finished".into(),
            },
            VerificationSpec::Command {
                command: "pytest tests/".into(),
                expected_exit_code: 0,
            },
            VerificationSpec::Custom {
                description: "Review deployment manually".into(),
            },
        ]
    );
    assert!(verifier.parse_specs(" \n ").is_empty());
}

#[tokio::test]
async fn test_verify_all_aggregates_results_and_rejects_empty_specs() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"));
    let verifier = TaskVerifier::new(workspace);
    let specs = vec![
        VerificationSpec::FileExists {
            path: "Cargo.toml".into(),
            min_bytes: Some(1),
        },
        VerificationSpec::Custom {
            description: "manual review required".into(),
        },
    ];

    let (passed, results) = verifier.verify_all(&specs).await;
    assert!(!passed);
    assert_eq!(results.len(), 2);
    assert!(results[0].passed);
    assert!(!results[1].passed);

    let (empty_passed, empty_results) = verifier.verify_all(&[]).await;
    assert!(!empty_passed);
    assert!(empty_results.is_empty());
}

#[tokio::test]
async fn test_verify_all_preserves_spec_order_in_results() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"));
    let verifier = TaskVerifier::new(workspace);
    let specs = vec![
        VerificationSpec::Custom {
            description: "first review".into(),
        },
        VerificationSpec::FileExists {
            path: "Cargo.toml".into(),
            min_bytes: Some(1),
        },
        VerificationSpec::Custom {
            description: "last review".into(),
        },
    ];

    let (_, results) = verifier.verify_all(&specs).await;

    assert_eq!(
        results
            .iter()
            .map(|result| &result.spec)
            .collect::<Vec<_>>(),
        specs.iter().collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn test_verify_command_spec_failure_handled_gracefully() {
    let temp_dir = std::env::temp_dir();
    let verifier = TaskVerifier::new(&temp_dir);
    let spec = VerificationSpec::Command {
        command: "exit 42".to_string(),
        expected_exit_code: 0,
    };
    let res = verifier.verify_spec(&spec).await;
    assert!(!res.passed);
    assert!(res.details.contains("42"));
}

#[tokio::test]
async fn test_verify_command_nonexistent_handled_gracefully() {
    let temp_dir = std::env::temp_dir();
    let verifier = TaskVerifier::new(&temp_dir);
    let spec = VerificationSpec::Command {
        command: "non_existent_binary_xyz_12345".to_string(),
        expected_exit_code: 0,
    };
    let res = verifier.verify_spec(&spec).await;
    assert!(!res.passed);
}

#[tokio::test]
async fn test_verify_command_rejected_when_shell_execution_is_disabled() {
    let temp_dir = std::env::temp_dir();
    let mut guardrail = agent_domain::GuardrailPolicy::default_enterprise();
    guardrail.allow_shell_execution = false;
    let verifier = TaskVerifier::with_policy(
        &temp_dir,
        crate::sandbox::SandboxConfig::default(),
        guardrail,
    );
    let spec = VerificationSpec::Command {
        command: "echo should-not-run".to_string(),
        expected_exit_code: 0,
    };

    let result = verifier.verify_spec(&spec).await;

    assert!(!result.passed);
    assert_eq!(result.spec, spec);
    assert_eq!(result.details, "Verification command rejected by policy");
}

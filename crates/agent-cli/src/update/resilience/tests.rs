use super::*;
use crate::fault::CliExecutionError;
use crate::update::TemporaryDirectory;
use std::path::PathBuf;

#[test]
fn builder_methods_store_repository_branch_and_destination() {
    let destination = PathBuf::from("relative/bin/agent-ctl");
    let updater = ResilientUpdater::default()
        .with_repository("https://example.invalid/agent.git")
        .with_branch("release/test")
        .with_destination(destination.clone());

    assert_eq!(updater.repository_url, "https://example.invalid/agent.git");
    assert_eq!(updater.branch, "release/test");
    assert_eq!(updater.destination.as_ref(), Some(&destination));

    let empty_values = ResilientUpdater::default()
        .with_repository("")
        .with_branch("");
    assert!(empty_values.repository_url.is_empty());
    assert!(empty_values.branch.is_empty());
}

#[test]
fn max_capacity_reports_unbounded_bounded_and_zero_limits() -> Result<(), CliExecutionError> {
    assert_eq!(ResilientUpdater::default().max_capacity(), None);
    assert_eq!(
        ResilientUpdater::default()
            .with_max_capacity(3)
            .max_capacity(),
        Some(3)
    );
    assert_eq!(
        ResilientUpdater::default()
            .with_max_capacity(0)
            .max_capacity(),
        Some(0)
    );

    let mut zero_capacity = ResilientUpdater::default().with_max_capacity(0);
    zero_capacity.execute_step("evicted-at-zero", || Ok(()))?;
    assert!(zero_capacity.runner().ledger().is_empty());
    assert!(zero_capacity.runner().recent_actions().is_empty());
    Ok(())
}

#[test]
fn runner_accessors_expose_and_mutate_the_updater_runner() {
    let mut updater = ResilientUpdater::default();
    assert_eq!(updater.runner().max_capacity(), None);

    let updated_runner = updater.runner().clone().with_max_capacity(7);
    *updater.runner_mut() = updated_runner;
    assert_eq!(updater.max_capacity(), Some(7));
}

#[test]
fn failed_and_rejected_counts_reflect_permanent_failures() {
    let mut updater = ResilientUpdater::default();
    let result = updater.execute_step("permanent-failure", || {
        Err::<(), _>(CliExecutionError::Other("invalid update input".to_string()))
    });

    assert!(result.is_err());
    assert_eq!(updater.failed_count(), 1);
    assert_eq!(updater.rejected_count(), 1);
}

#[test]
fn reset_counts_clears_counters_but_preserves_audit_history() -> Result<(), CliExecutionError> {
    let mut updater = ResilientUpdater::default();
    updater.execute_step("completed-before-reset", || Ok(()))?;
    let ledger_len = updater.runner().ledger().len();
    let recent_len = updater.runner().recent_actions().len();
    assert_eq!(updater.total_executed(), 1);

    updater.reset_counts();

    assert_eq!(updater.total_executed(), 0);
    assert_eq!(updater.recovered_count(), 0);
    assert_eq!(updater.failed_count(), 0);
    assert_eq!(updater.rejected_count(), 0);
    assert_eq!(updater.panics_caught_count(), 0);
    assert_eq!(updater.runner().ledger().len(), ledger_len);
    assert_eq!(updater.runner().recent_actions().len(), recent_len);
    Ok(())
}

#[test]
fn build_release_returns_error_when_source_has_no_cargo_manifest() -> Result<(), CliExecutionError>
{
    let source = TemporaryDirectory::new(&std::env::temp_dir())?;
    let target = source.path().join("target");
    let mut updater = ResilientUpdater::default();

    let result = updater.build_release(source.path(), &target);

    assert!(result.is_err());
    assert_eq!(updater.failed_count(), 1);
    Ok(())
}

#[test]
fn temporary_directory_creation_reports_io_error_for_missing_parent() {
    let file_parent =
        std::env::temp_dir().join(format!("agent-cli-file-parent-{}", uuid::Uuid::new_v4()));
    assert!(std::fs::write(&file_parent, b"not a directory").is_ok());
    let invalid_parent = file_parent.join("child");

    let result = TemporaryDirectory::new(&invalid_parent);
    let cleanup = std::fs::remove_file(file_parent);

    assert!(cleanup.is_ok());
    assert!(matches!(result, Err(CliExecutionError::TransientIo(_))));
}

use agent_cli::cli::Cli;
use agent_cli::{CliExecutionError, CliFaultPolicy, ResilientCliRunner, ShardedResilientCliRunner};

#[test]
fn sync_runner_executes_and_reset_counts_preserves_audit() {
    let mut runner = ResilientCliRunner::default().with_max_retries(1);
    runner.set_policy(CliFaultPolicy::none());
    assert_eq!(runner.run_resilient_sync("sync_ok", || Ok(7)), Ok(7));
    assert_eq!(runner.total_executed(), 1);
    assert_eq!(
        runner.recent_actions().back().map(String::as_str),
        Some("sync_ok")
    );

    runner.reset_counts();
    assert_eq!(runner.total_executed(), 0);
    assert_eq!(runner.recovered_count(), 0);
    assert_eq!(runner.failed_count(), 0);
    assert_eq!(runner.panics_caught_count(), 0);
    assert_eq!(runner.ledger().len(), 1);
}

#[tokio::test]
async fn run_batch_operations_returns_each_outcome_and_updates_counts() {
    let mut runner = ResilientCliRunner::default();
    let operations = [
        ("first", Ok("one")),
        (
            "bad_input",
            Err(CliExecutionError::ParseError {
                code: 2,
                message: "invalid input".into(),
            }),
        ),
        ("last", Ok("three")),
    ];

    let results = runner.run_batch_operations(&operations).await;
    assert_eq!(results.len(), 3);
    assert_eq!(results[0], Ok("one"));
    assert_eq!(
        results[1].as_ref().unwrap_err(),
        &CliExecutionError::ParseError {
            code: 2,
            message: "invalid input".into(),
        }
    );
    assert_eq!(results[2], Ok("three"));
    assert_eq!(runner.total_executed(), 2);
    assert_eq!(runner.failed_count(), 1);
    assert_eq!(runner.rejected_count(), 1);
}

#[tokio::test]
async fn execute_batch_commands_dispatches_cli_commands() {
    let mut runner = ResilientCliRunner::default();
    let cli = Cli::try_parse_resilient(["agent-ctl", "list"]).expect("list command parses");

    let results = runner.execute_batch_commands(&[cli]).await;
    assert_eq!(results, vec![Ok(())]);
    assert_eq!(runner.total_executed(), 1);
}

#[test]
fn bounded_runner_reports_capacity_and_tracks_memory() {
    let mut runner = ResilientCliRunner::default().with_max_capacity(1);
    assert!(runner.is_bounded());
    assert_eq!(runner.max_capacity(), Some(1));
    assert!(runner.memory_usage_bytes() >= std::mem::size_of::<ResilientCliRunner>());

    runner.run_resilient_sync("first", || Ok(())).unwrap();
    runner.run_resilient_sync("second", || Ok(())).unwrap();
    assert_eq!(runner.recent_actions().len(), 1);
    assert_eq!(
        runner.recent_actions().front().map(String::as_str),
        Some("second")
    );
    assert_eq!(runner.ledger().len(), 1);
}

#[test]
fn zero_capacity_runner_evicts_audit_and_recent_actions() {
    let mut runner = ResilientCliRunner::default().with_max_capacity(0);
    runner.run_resilient_sync("discarded", || Ok(())).unwrap();
    assert!(runner.is_bounded());
    assert_eq!(runner.max_capacity(), Some(0));
    assert!(runner.recent_actions().is_empty());
    assert!(runner.ledger().is_empty());
}

#[test]
fn policy_builder_and_getter_expose_the_configured_fault_policy() {
    let policy = CliFaultPolicy::always(agent_cli::CliFault::NonZeroExit(3));
    let mut runner = ResilientCliRunner::default()
        .with_policy(policy)
        .with_max_retries(0);

    assert!(format!("{:?}", runner.policy()).contains("NonZeroExit(3)"));
    assert_eq!(
        runner.run_resilient_sync("configured_fault", || Ok(())),
        Err(CliExecutionError::NonZeroExit {
            code: 3,
            message: "Mission exited with non-zero code 3".into(),
        })
    );
    assert_eq!(runner.failed_count(), 1);
}

#[test]
fn set_policy_replaces_the_policy_after_runner_creation() {
    let mut runner = ResilientCliRunner::new(
        CliFaultPolicy::always(agent_cli::CliFault::NonZeroExit(3)),
        0,
    );
    runner.set_policy(CliFaultPolicy::none());

    assert_eq!(
        runner.run_resilient_sync("policy_replaced", || Ok(9)),
        Ok(9)
    );
    assert_eq!(runner.total_executed(), 1);
    assert_eq!(runner.failed_count(), 0);
}

#[tokio::test]
async fn run_command_resilient_dispatches_one_valid_command() {
    let mut runner = ResilientCliRunner::default();
    let cli = Cli::try_parse_resilient(["agent-ctl", "list"]).expect("list command parses");

    assert_eq!(runner.run_command_resilient(cli).await, Ok(()));
    assert_eq!(runner.total_executed(), 1);
    assert_eq!(
        runner.recent_actions().back().map(String::as_str),
        Some("cli_command_dispatch")
    );
}

#[tokio::test]
async fn empty_batch_operations_returns_no_results_and_does_not_change_metrics() {
    let mut runner = ResilientCliRunner::default();

    let results: Vec<Result<u8, CliExecutionError>> = runner.execute_batch_operations(&[]).await;

    assert!(results.is_empty());
    assert_eq!(runner.total_executed(), 0);
    assert_eq!(runner.failed_count(), 0);
    assert!(runner.ledger().is_empty());
}

#[tokio::test]
async fn sharded_audit_workspace_persists_executed_action() {
    let workspace = std::env::temp_dir().join(format!(
        "agent-cli-sharded-audit-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&workspace).expect("create audit workspace");
    let runner = ShardedResilientCliRunner::new(CliFaultPolicy::none(), 1, 2)
        .with_audit_workspace(&workspace);

    assert_eq!(
        runner
            .run_resilient("persisted_action", || async { Ok(()) })
            .await,
        Ok(())
    );
    let audit: serde_json::Value = serde_json::from_slice(
        &std::fs::read(workspace.join(".audit_ledger.json")).expect("audit file is written"),
    )
    .expect("audit file contains JSON");
    assert!(audit.to_string().contains("persisted_action"));

    std::fs::remove_dir_all(workspace).expect("remove temporary audit workspace");
}

#[tokio::test]
async fn shard_access_wraps_out_of_range_indices() {
    let runner = ShardedResilientCliRunner::new(CliFaultPolicy::none(), 1, 3);

    assert_eq!(runner.num_shards(), 3);
    assert!(std::ptr::eq(
        runner.shard(usize::MAX),
        runner.shard(usize::MAX % 3)
    ));
    assert!(std::ptr::eq(
        runner.shard_for(""),
        runner.shard(runner.shard_idx(""))
    ));
}

#[tokio::test]
async fn cloned_sharded_runner_shares_execution_state() {
    let runner = ShardedResilientCliRunner::new(CliFaultPolicy::none(), 1, 2);
    let clone = runner.clone();

    assert_eq!(
        clone
            .run_resilient("shared_action", || async { Ok(()) })
            .await,
        Ok(())
    );
    assert_eq!(runner.total_executed().await, 1);
    assert_eq!(clone.total_executed().await, 1);
}

#[tokio::test]
async fn sharded_runner_normalizes_shards_and_routes_keys_consistently() {
    let runner = ShardedResilientCliRunner::new(CliFaultPolicy::none(), 2, 0).with_max_capacity(8);
    assert_eq!(runner.num_shards(), 16);
    assert_eq!(runner.total_capacity(), Some(8));
    assert_eq!(
        runner.shard_idx("stable-key"),
        runner.shard_idx("stable-key")
    );
    assert_eq!(
        runner.shard_for("stable-key") as *const _,
        runner.shard(runner.shard_idx("stable-key")) as *const _
    );

    let shard = runner.shard(usize::MAX).lock().await;
    // Capacity is divided per shard with a minimum allocation plus one slot.
    assert_eq!(shard.max_capacity(), Some(2));
}

#[tokio::test]
async fn sharded_runner_aggregates_metrics_memory_and_reset() {
    let runner = ShardedResilientCliRunner::new(CliFaultPolicy::none(), 2, 4);
    assert_eq!(
        runner.run_resilient("aggregate", || async { Ok(()) }).await,
        Ok(())
    );
    assert_eq!(runner.total_executed().await, 1);
    assert_eq!(runner.recovered_count().await, 0);
    assert_eq!(runner.failed_count().await, 0);
    assert_eq!(runner.panics_caught_count().await, 0);
    assert!(runner.memory_usage_bytes().await > 0);

    runner.reset_counts().await;
    assert_eq!(runner.total_executed().await, 0);
}

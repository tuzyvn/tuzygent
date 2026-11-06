use agent_domain::GuardrailPolicy;
use agent_engine::cli::MockEngine;
use agent_memory::{DurableMemoryStore, ScratchpadManager};
use agent_runtime::ooda::execution::runner::step::{
    compact_working_context_with_budget, estimate_tokens, resolve_active_system_context,
    resolve_active_system_context_with_budget, DEFAULT_WORKING_CONTEXT_TOKEN_BUDGET,
};
use agent_runtime::ooda::execution::SubtaskExecutor;
use agent_runtime::supplement::InFlightSupplementController;
use std::sync::Arc;

#[test]
fn test_default_working_context_token_budget_constant() {
    assert_eq!(DEFAULT_WORKING_CONTEXT_TOKEN_BUDGET, 2000);
}

#[test]
fn test_estimate_tokens_calculation() {
    assert_eq!(estimate_tokens(""), 0);
    assert_eq!(estimate_tokens("test"), 1); // 4 chars -> 1 token
    assert_eq!(estimate_tokens("tests"), 2); // 5 chars -> 2 tokens
    assert_eq!(estimate_tokens(&"a".repeat(400)), 100); // 400 chars -> 100 tokens
}

#[test]
fn test_compact_working_context_within_budget_retains_all() {
    let ctx = "ST-1: successfully compiled\nST-2: tests passed";
    let budget = 100; // 400 chars, ctx is ~50 chars
    let compacted = compact_working_context_with_budget(ctx, budget);
    assert_eq!(compacted, ctx);
}

#[test]
fn test_compact_working_context_zero_budget_returns_empty() {
    let ctx = "ST-1: some long result from previous subtask";
    let compacted = compact_working_context_with_budget(ctx, 0);
    assert_eq!(compacted, "");
}

#[test]
fn test_compact_working_context_rolling_compaction_exceeding_budget() {
    // Generate 5 subtask steps with long excerpts
    let mut ctx = String::new();
    for i in 1..=5 {
        ctx.push_str(&format!(
            "\nST-{}: Executed subtask step {} with detailed output: {}\n",
            i,
            i,
            "x".repeat(120)
        ));
    }

    let initial_tokens = estimate_tokens(&ctx);
    assert!(
        initial_tokens > 150,
        "Initial context should have > 150 tokens, got {}",
        initial_tokens
    );

    // Set budget to 60 tokens (~240 chars)
    let budget = 60;
    let compacted = compact_working_context_with_budget(&ctx, budget);

    let compacted_tokens = estimate_tokens(&compacted);
    assert!(
        compacted_tokens <= budget,
        "Compacted tokens ({}) must not exceed budget ({})",
        compacted_tokens,
        budget
    );

    // The most recent subtask (ST-5) should be retained in full or prioritized
    assert!(
        compacted.contains("ST-5"),
        "Most recent subtask ST-5 should be present in compacted context"
    );

    // Earlier subtasks should be compacted / summarized
    assert!(
        compacted.contains("compacted") || compacted.contains("Summarized"),
        "Should contain rolling compaction summary notice"
    );
}

#[test]
fn test_compact_working_context_extreme_tight_budget() {
    let mut ctx = String::new();
    for i in 1..=10 {
        ctx.push_str(&format!("\nST-{}: output data {}\n", i, "data ".repeat(20)));
    }

    let tight_budget = 15; // 60 chars maximum
    let compacted = compact_working_context_with_budget(&ctx, tight_budget);
    let compacted_tokens = estimate_tokens(&compacted);

    assert!(
        compacted_tokens <= tight_budget,
        "Even under extreme tight budget, tokens ({}) must <= budget ({})",
        compacted_tokens,
        tight_budget
    );
}

#[test]
fn test_compact_working_context_unicode_safe() {
    let ctx = "\nST-1: 🚀 Khởi động worker thành công với thông số chuẩn xác.\n\
               \nST-2: 🔍 Kiểm tra tính toàn vẹn dữ liệu: không có lỗi phát sinh.\n\
               \nST-3: 🎯 Hoàn thành mục tiêu nghiên cứu với độ chính xác cao.\n";

    let budget = 30; // ~120 chars
    let compacted = compact_working_context_with_budget(ctx, budget);
    let compacted_tokens = estimate_tokens(&compacted);

    assert!(compacted_tokens <= budget);
    assert!(compacted.contains("ST-3"));
}

#[tokio::test]
async fn test_resolve_active_system_context_with_budget_integration() {
    let temp_dir = std::env::temp_dir().join(format!("test_compact_ctx_{}", uuid::Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let scratchpad = ScratchpadManager::new(temp_dir.join("scratchpad"));
    let durable = DurableMemoryStore::new(temp_dir.join("durable.db"));
    let ctrl = InFlightSupplementController::new(5);

    let executor = SubtaskExecutor::new(
        Arc::new(MockEngine::new("")),
        GuardrailPolicy::default_enterprise(),
        scratchpad,
        durable,
        temp_dir.to_string_lossy().to_string(),
        ctrl,
        None,
        None,
        30,
    );

    let long_term = "LONG_TERM_SYSTEM_GOAL: Fix all vulnerabilities";
    let mut huge_working_context = String::new();
    for i in 1..=10 {
        huge_working_context.push_str(&format!(
            "\nST-{}: Step {} executed command with output: {}\n",
            i,
            i,
            "result_payload ".repeat(30)
        ));
    }

    // Default resolution uses DEFAULT_WORKING_CONTEXT_TOKEN_BUDGET
    let resolved_default =
        resolve_active_system_context(&executor, long_term, &huge_working_context).await;
    assert!(resolved_default.contains("LONG_TERM_SYSTEM_GOAL"));
    assert!(resolved_default.contains("# Previous step results"));

    // Explicit small budget resolution
    let small_budget = 50;
    let resolved_compact = resolve_active_system_context_with_budget(
        &executor,
        long_term,
        &huge_working_context,
        small_budget,
    )
    .await;

    assert!(resolved_compact.contains("LONG_TERM_SYSTEM_GOAL"));
    assert!(resolved_compact.contains("ST-10"));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

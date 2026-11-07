use crate::ooda::execution::SubtaskExecutor;
use crate::supplement::InFlightSupplementController;
use agent_domain::GuardrailPolicy;
use agent_engine::ComputeEngine;
use agent_memory::{DurableMemoryStore, ScratchpadManager};
use anyhow::Context;
use std::path::Path;
use std::sync::Arc;

pub fn try_setup_test_executor(
    engine: Arc<dyn ComputeEngine>,
    guardrail: Option<GuardrailPolicy>,
    workspace_dir: &Path,
    timeout_secs: Option<u64>,
) -> anyhow::Result<SubtaskExecutor> {
    if !workspace_dir.exists() {
        std::fs::create_dir_all(workspace_dir).with_context(|| {
            format!(
                "Failed to prepare workspace_dir at {}",
                workspace_dir.display()
            )
        })?;
    }
    let guardrail = guardrail.unwrap_or_default();
    let scratchpad = ScratchpadManager::new(workspace_dir.join("scratchpad"));
    let durable = DurableMemoryStore::new(workspace_dir.join("durable.db"));
    let ctrl = InFlightSupplementController::new(1);
    let timeout = match timeout_secs {
        Some(0) | None => 30,
        Some(t) => t,
    };

    Ok(SubtaskExecutor::new(
        engine,
        guardrail,
        scratchpad,
        durable,
        workspace_dir.to_string_lossy().to_string(),
        ctrl,
        None,
        None,
        timeout,
    ))
}

pub fn setup_test_executor_with_timeout(
    engine: Arc<dyn ComputeEngine>,
    guardrail: Option<GuardrailPolicy>,
    workspace_dir: &Path,
    timeout_secs: u64,
) -> SubtaskExecutor {
    let effective_timeout = if timeout_secs == 0 { 30 } else { timeout_secs };
    match try_setup_test_executor(
        engine.clone(),
        guardrail.clone(),
        workspace_dir,
        Some(effective_timeout),
    ) {
        Ok(executor) => executor,
        Err(_) => {
            let guardrail = guardrail.unwrap_or_default();
            let scratchpad = ScratchpadManager::new(workspace_dir.join("scratchpad"));
            let durable = DurableMemoryStore::new(workspace_dir.join("durable.db"));
            let ctrl = InFlightSupplementController::new(1);
            SubtaskExecutor::new(
                engine,
                guardrail,
                scratchpad,
                durable,
                workspace_dir.to_string_lossy().to_string(),
                ctrl,
                None,
                None,
                effective_timeout,
            )
        }
    }
}

pub fn setup_test_executor(
    engine: Arc<dyn ComputeEngine>,
    guardrail: Option<GuardrailPolicy>,
    workspace_dir: &Path,
) -> SubtaskExecutor {
    setup_test_executor_with_timeout(engine, guardrail, workspace_dir, 30)
}

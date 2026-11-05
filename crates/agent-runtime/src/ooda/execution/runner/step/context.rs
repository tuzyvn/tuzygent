use crate::ooda::execution::SubtaskExecutor;
use tracing::info;

/// Incorporates in-flight supplements into system context if available.
pub async fn resolve_active_system_context(
    executor: &SubtaskExecutor,
    long_term_ctx: &str,
    working_context: &str,
) -> String {
    let mut active_system_context = format!(
        "{}\n# Previous step results (reference data)\n{}",
        long_term_ctx, working_context
    );
    if let Some(sup) = executor.supplement_ctrl.try_recv().await {
        info!(
            "💡 [OODA] Incorporating In-Flight Supplement from: {}",
            sup.author_name
        );
        active_system_context.push_str("\n\n");
        active_system_context.push_str(&sup.to_instruction_block());
    }
    active_system_context
}

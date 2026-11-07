use super::ResilientExecutionService;
use agent_domain::{ActionSeverity, AuditEventType, AuditLedger};
use agent_engine::{ComputeEngine, EngineError, EngineInput, EngineOutput};
use std::sync::Arc;
impl ResilientExecutionService {
    /// Resilient execution with integrated audit ledger recording
    pub async fn execute_and_record(
        &mut self,
        ledger: &mut AuditLedger,
        engine: Arc<dyn ComputeEngine>,
        input: &EngineInput,
    ) -> Result<EngineOutput, EngineError> {
        let command_summary = if input.prompt_or_command.len() > 60 {
            format!("{}...", &input.prompt_or_command[..60])
        } else {
            input.prompt_or_command.clone()
        };

        let result = self.execute_resilient(engine, input).await;
        match &result {
            Ok(output) => {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    ledger.record_event(
                        AuditEventType::CommandAllowed,
                        ActionSeverity::Routine,
                        format!(
                            "ExecutionService command executed successfully [exit_code: {}]: {}",
                            output.exit_code, command_summary
                        ),
                    );
                }));
            }
            Err(err) => {
                let (event_type, severity) = match err {
                    EngineError::PolicyRejected(_) => {
                        (AuditEventType::CommandBlocked, ActionSeverity::Destructive)
                    }
                    _ => (AuditEventType::CommandException, ActionSeverity::HighImpact),
                };
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    ledger.record_event(
                        event_type,
                        severity,
                        format!(
                            "ExecutionService failed for command '{}': {}",
                            command_summary, err
                        ),
                    );
                }));
            }
        }

        result
    }
}

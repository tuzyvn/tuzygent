use super::ResilientExecutionService;
use crate::execution::{is_retryable_execution_error, ExecutionFault};
use agent_engine::{ComputeEngine, EngineError, EngineInput, EngineOutput};
use std::sync::Arc;
impl ResilientExecutionService {
    pub async fn execute_batch(
        &mut self,
        engine: Arc<dyn ComputeEngine>,
        inputs: &[EngineInput],
    ) -> Vec<Result<EngineOutput, EngineError>> {
        let mut results = Vec::with_capacity(inputs.len());
        for input in inputs {
            results.push(self.execute_resilient(Arc::clone(&engine), input).await);
        }
        results
    }

    /// Execute the compute engine with resilience, retrying on transient errors and isolating panics
    pub async fn execute_resilient(
        &mut self,
        engine: Arc<dyn ComputeEngine>,
        input: &EngineInput,
    ) -> Result<EngineOutput, EngineError> {
        // Red line policy violations are non-retryable and must fail immediately
        if !input.prompt_or_command.is_empty() {
            if let Err(red_line_err) = self
                .service
                .guardrail()
                .check_command_safety(&input.prompt_or_command)
            {
                self.failed_count += 1;
                return Err(EngineError::PolicyRejected(red_line_err));
            }
        }

        let mut attempt = 0;
        loop {
            attempt += 1;

            // Check if a simulated chaos fault should be injected
            if let Some(fault) = self.policy.should_inject() {
                if fault == ExecutionFault::Panic {
                    self.panics_caught_count += 1;
                    if attempt <= self.max_retries {
                        self.recovered_count += 1;
                        if let Some(ref policy) = self.backoff_policy {
                            policy.sleep_for_attempt(attempt as u32).await;
                        }
                        continue;
                    }
                    self.failed_count += 1;
                    return Err(fault.to_engine_error());
                }

                if attempt <= self.max_retries {
                    self.recovered_count += 1;
                    if let Some(ref policy) = self.backoff_policy {
                        policy.sleep_for_attempt(attempt as u32).await;
                    }
                    continue;
                }

                self.failed_count += 1;
                return Err(fault.to_engine_error());
            }

            // Spawn execution in task boundary to isolate potential panics from the worker
            let srv = self.service.clone();
            let eng = engine.clone();
            let inp = input.clone();
            let task_result = tokio::spawn(async move { srv.execute(eng, &inp).await }).await;

            match task_result {
                Ok(Ok(output)) => {
                    self.total_executed += 1;
                    self.cache_response(&input.prompt_or_command, &output);
                    return Ok(output);
                }
                Ok(Err(engine_err)) => {
                    if let EngineError::ProcessFailed(ref msg) = engine_err {
                        if msg.contains("panicked") {
                            self.panics_caught_count += 1;
                        }
                    }

                    if !is_retryable_execution_error(&engine_err) {
                        self.failed_count += 1;
                        return Err(engine_err);
                    }

                    if attempt <= self.max_retries {
                        self.recovered_count += 1;
                        if let Some(ref policy) = self.backoff_policy {
                            policy.sleep_for_attempt(attempt as u32).await;
                        }
                        continue;
                    }

                    self.failed_count += 1;
                    return Err(engine_err);
                }
                Err(join_err) => {
                    if join_err.is_panic() {
                        self.panics_caught_count += 1;
                        if attempt <= self.max_retries {
                            self.recovered_count += 1;
                            if let Some(ref policy) = self.backoff_policy {
                                policy.sleep_for_attempt(attempt as u32).await;
                            }
                            continue;
                        }
                        self.failed_count += 1;
                        return Err(EngineError::ProcessFailed(
                            "Execution worker panicked unexpectedly".to_string(),
                        ));
                    }

                    self.failed_count += 1;
                    return Err(EngineError::ProcessFailed(format!(
                        "Execution task failed: {}",
                        join_err
                    )));
                }
            }
        }
    }
}

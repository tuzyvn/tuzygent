use agent_domain::{VerificationResult, VerificationSpec};
use tracing::warn;

use super::{format::spec_description, TaskVerifier};

impl TaskVerifier<'_> {
    /// Verifies all specifications for a subtask
    pub async fn verify_all(&self, specs: &[VerificationSpec]) -> (bool, Vec<VerificationResult>) {
        let mut results = Vec::new();
        let mut all_passed = !specs.is_empty();
        for spec in specs {
            let res = self.verify_spec(spec).await;
            if !res.passed {
                warn!(
                    "Verification check failed: {} -> {}",
                    spec_description(spec),
                    res.details
                );
                all_passed = false;
            }
            results.push(res);
        }
        (all_passed, results)
    }
}

use super::ResilientAuthValidator;
use crate::auth::fault::AuthFaultPolicy;
use crate::auth::types::AuthError;

impl ResilientAuthValidator {
    pub fn max_capacity(&self) -> Option<usize> {
        self.max_capacity
    }

    pub fn memory_usage_bytes(&self) -> usize {
        let base = std::mem::size_of::<Self>();
        let token_bytes = self.token.len() + self.fallback_token.as_ref().map_or(0, |t| t.len());
        let hist_bytes: usize = self
            .history
            .iter()
            .map(|s| s.len() + std::mem::size_of::<String>())
            .sum();
        base + token_bytes + hist_bytes
    }

    pub fn reset_counts(&mut self) {
        self.validated_count = 0;
        self.recovered_count = 0;
        self.rejected_count = 0;
        self.fallback_count = 0;
    }

    pub(super) fn record_history(&mut self, candidate: &str) {
        if let Some(cap) = self.max_capacity {
            if self.history.len() >= cap {
                self.history.pop_front();
            }
            self.history.push_back(candidate.to_string());
        }
    }

    pub fn increment_validated(&mut self) {
        self.validated_count = self.validated_count.saturating_add(1);
    }

    pub fn increment_rejected(&mut self) {
        self.rejected_count = self.rejected_count.saturating_add(1);
    }

    pub fn validated_count(&self) -> usize {
        self.validated_count
    }

    pub fn recovered_count(&self) -> usize {
        self.recovered_count
    }

    pub fn rejected_count(&self) -> usize {
        self.rejected_count
    }

    pub fn fallback_count(&self) -> usize {
        self.fallback_count
    }

    pub fn fault_policy(&self) -> &AuthFaultPolicy {
        &self.fault_policy
    }

    pub fn fault_policy_mut(&mut self) -> &mut AuthFaultPolicy {
        &mut self.fault_policy
    }

    pub fn is_retryable_error(&self, err: &AuthError) -> bool {
        err.is_retryable()
    }
}

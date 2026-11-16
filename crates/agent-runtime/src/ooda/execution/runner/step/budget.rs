use serde::{Deserialize, Serialize};

/// Dynamic multi-layer context budget derived from model context window.
///
/// Inspired by Orkas's adaptive compaction budgeting:
/// - Reserves 1/6 of room for growth between steps without stalling.
/// - Allocates 60% of active headroom to working context and 40% to history.
/// - Scales summary target dynamically (2% of active room, clamped between 1,200 and 6,000 tokens).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextBudget {
    /// Total usable model context window tokens.
    pub max_model_window_tokens: usize,
    /// Headroom allocated to the active working context.
    pub working_context_tokens: usize,
    /// Reserve room allocated to long-term / history context.
    pub history_room_tokens: usize,
    /// Planning buffer reserved for intermediate tool output growth.
    pub inline_growth_tokens: usize,
    /// Dynamic token ceiling for subtask compaction summaries.
    pub summary_target_tokens: usize,
}

impl ContextBudget {
    /// Derive a proportional context budget from an explicit model window token count.
    pub fn for_window(window_tokens: usize) -> Self {
        // Reserve 1,000 tokens for system prompt framing, safety rules, and personas
        let usable = window_tokens.saturating_sub(1000);
        // Reserve one-sixth of usable headroom for tool output growth
        let inline_growth = usable / 6;
        let rest = usable.saturating_sub(inline_growth);

        // 60% active working share, 40% history share
        let working_context = (rest as f64 * 0.6) as usize;
        let history_room = (rest as f64 * 0.4) as usize;

        // Dynamic summary target: 2% of working context share, clamped between 1,200 and 6,000 tokens
        let summary_target = (working_context as f64 * 0.02).clamp(1200.0, 6000.0) as usize;

        Self {
            max_model_window_tokens: window_tokens,
            // Ensure minimum baseline of 2,000 tokens
            working_context_tokens: working_context.max(2000),
            history_room_tokens: history_room,
            inline_growth_tokens: inline_growth,
            summary_target_tokens: summary_target,
        }
    }

    /// Standard enterprise default baseline (128K context window).
    pub fn default_enterprise() -> Self {
        Self::for_window(128_000)
    }

    /// Minimal budget for constrained or small local models (e.g. 8K tokens).
    pub fn minimal_local() -> Self {
        Self::for_window(8_192)
    }

    /// Budget for large reasoning models (e.g. 200K tokens).
    pub fn large_window() -> Self {
        Self::for_window(200_000)
    }
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self::default_enterprise()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_budget_for_128k_window() {
        let budget = ContextBudget::default_enterprise();
        assert_eq!(budget.max_model_window_tokens, 128_000);
        assert!(budget.working_context_tokens > 50_000, "128k window working context should be generous");
        assert!(budget.summary_target_tokens >= 1200);
        assert!(budget.inline_growth_tokens > 10_000);
    }

    #[test]
    fn test_budget_for_small_window_preserves_floor() {
        let budget = ContextBudget::for_window(4_000);
        assert_eq!(budget.max_model_window_tokens, 4_000);
        assert_eq!(budget.working_context_tokens, 2000, "Should preserve 2000 floor");
        assert_eq!(budget.summary_target_tokens, 1200, "Should clamp to minimum 1200");
    }

    #[test]
    fn test_budget_proportions_are_consistent() {
        let budget = ContextBudget::for_window(64_000);
        assert!(budget.working_context_tokens > budget.history_room_tokens);
        assert!(budget.inline_growth_tokens > 0);
        assert!(budget.working_context_tokens + budget.history_room_tokens + budget.inline_growth_tokens <= 64_000);
    }
}

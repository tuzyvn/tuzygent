use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Full Jitter Exponential Backoff Policy for engine execution retry.
///
/// Implements exponential backoff with full jitter to avoid thundering herd problems
/// when LLM engines encounter transient rate limits (HTTP 429) or service outages (HTTP 503).
///
/// Formula for Full Jitter (AWS Architecture Pattern):
///   `t = min(max_interval, initial_interval * multiplier^(attempt - 1))`
///   `sleep = rand(0, t)` if jitter is enabled, else `t`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackoffPolicy {
    pub initial_interval: Duration,
    pub max_interval: Duration,
    pub multiplier: f64,
    pub jitter: bool,
}

impl Default for BackoffPolicy {
    fn default() -> Self {
        Self {
            initial_interval: Duration::from_millis(25),
            max_interval: Duration::from_millis(1000),
            multiplier: 2.0,
            jitter: true,
        }
    }
}

impl BackoffPolicy {
    /// Creates a new `BackoffPolicy` with the given parameters.
    pub fn new(
        initial_interval: Duration,
        max_interval: Duration,
        multiplier: f64,
        jitter: bool,
    ) -> Self {
        Self {
            initial_interval,
            max_interval,
            multiplier: if multiplier < 1.0 { 1.0 } else { multiplier },
            jitter,
        }
    }

    /// Creates a no-op backoff policy (zero delay, no jitter).
    pub fn none() -> Self {
        Self {
            initial_interval: Duration::ZERO,
            max_interval: Duration::ZERO,
            multiplier: 1.0,
            jitter: false,
        }
    }

    /// Creates a fixed-delay backoff policy.
    pub fn fixed(interval: Duration) -> Self {
        Self {
            initial_interval: interval,
            max_interval: interval,
            multiplier: 1.0,
            jitter: false,
        }
    }

    /// Creates a standard exponential backoff policy with jitter enabled.
    pub fn exponential(initial: Duration, max: Duration, multiplier: f64) -> Self {
        Self::new(initial, max, multiplier, true)
    }

    pub fn with_initial_interval(mut self, interval: Duration) -> Self {
        self.initial_interval = interval;
        self
    }

    pub fn with_max_interval(mut self, interval: Duration) -> Self {
        self.max_interval = interval;
        self
    }

    pub fn with_multiplier(mut self, multiplier: f64) -> Self {
        self.multiplier = if multiplier < 1.0 { 1.0 } else { multiplier };
        self
    }

    pub fn with_jitter(mut self, jitter: bool) -> Self {
        self.jitter = jitter;
        self
    }

    pub fn initial_interval(&self) -> Duration {
        self.initial_interval
    }

    pub fn max_interval(&self) -> Duration {
        self.max_interval
    }

    pub fn multiplier(&self) -> f64 {
        self.multiplier
    }

    pub fn jitter(&self) -> bool {
        self.jitter
    }

    /// Calculates the maximum theoretical delay for a given attempt number (1-indexed),
    /// bounded strictly between `initial_interval` and `max_interval`.
    pub fn calculate_max_delay(&self, attempt: u32) -> Duration {
        if self.initial_interval.is_zero() || self.max_interval.is_zero() {
            return Duration::ZERO;
        }
        let exp = attempt.saturating_sub(1).min(30);
        let factor = self.multiplier.powi(exp as i32);
        let millis = (self.initial_interval.as_millis() as f64 * factor) as u64;
        let delay = Duration::from_millis(millis);
        delay.min(self.max_interval)
    }

    /// Alias for `calculate_max_delay`.
    pub fn calculate_exponential_delay(&self, attempt: u32) -> Duration {
        self.calculate_max_delay(attempt)
    }

    /// Computes delay for a specific attempt given a deterministic ratio in `[0.0, 1.0]`.
    pub fn compute_delay_with_ratio(&self, attempt: u32, ratio: f64) -> Duration {
        let max_delay = self.calculate_max_delay(attempt);
        if !self.jitter || max_delay.is_zero() {
            return max_delay;
        }
        let clamped = ratio.clamp(0.0, 1.0);
        let millis = (max_delay.as_millis() as f64 * clamped) as u64;
        Duration::from_millis(millis)
    }

    /// Computes the actual delay for an attempt, applying Full Jitter if enabled.
    pub fn compute_delay(&self, attempt: u32) -> Duration {
        let max_delay = self.calculate_max_delay(attempt);
        if !self.jitter || max_delay.is_zero() {
            return max_delay;
        }
        let ratio = pseudo_random_fraction();
        let millis = (max_delay.as_millis() as f64 * ratio) as u64;
        Duration::from_millis(millis)
    }

    /// Computes the delay and sleeps asynchronously for that duration.
    pub async fn sleep_for_attempt(&self, attempt: u32) -> Duration {
        let delay = self.compute_delay(attempt);
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        delay
    }
}

/// Thread-safe SplitMix64 pseudo-random generator returning values in [0.0, 1.0).
fn pseudo_random_fraction() -> f64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::SystemTime;

    static COUNTER: AtomicU64 = AtomicU64::new(0x8543_1245_9876_abcf);
    let count = COUNTER.fetch_add(0x9e37_79b9_7f4a_7c15, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut z = nanos.wrapping_add(count);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z = z ^ (z >> 31);
    (z as f64) / (u64::MAX as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backoff_policy_builder_and_getters() {
        let policy = BackoffPolicy::new(
            Duration::from_millis(50),
            Duration::from_millis(800),
            2.5,
            false,
        );
        assert_eq!(policy.initial_interval(), Duration::from_millis(50));
        assert_eq!(policy.max_interval(), Duration::from_millis(800));
        assert_eq!(policy.multiplier(), 2.5);
        assert!(!policy.jitter());

        let policy2 = policy
            .with_initial_interval(Duration::from_millis(100))
            .with_max_interval(Duration::from_millis(1000))
            .with_multiplier(3.0)
            .with_jitter(true);

        assert_eq!(policy2.initial_interval(), Duration::from_millis(100));
        assert_eq!(policy2.max_interval(), Duration::from_millis(1000));
        assert_eq!(policy2.multiplier(), 3.0);
        assert!(policy2.jitter());
    }

    #[test]
    fn test_exponential_delay_calculation_without_jitter() {
        let policy = BackoffPolicy::new(
            Duration::from_millis(100),
            Duration::from_millis(1000),
            2.0,
            false,
        );
        // Attempt 0 and 1 are initial interval
        assert_eq!(policy.calculate_max_delay(0), Duration::from_millis(100));
        assert_eq!(policy.calculate_max_delay(1), Duration::from_millis(100));
        assert_eq!(policy.compute_delay(1), Duration::from_millis(100));

        // Attempt 2: 100 * 2 = 200ms
        assert_eq!(policy.calculate_max_delay(2), Duration::from_millis(200));
        assert_eq!(policy.compute_delay(2), Duration::from_millis(200));

        // Attempt 3: 100 * 4 = 400ms
        assert_eq!(policy.calculate_max_delay(3), Duration::from_millis(400));
        assert_eq!(policy.compute_delay(3), Duration::from_millis(400));

        // Attempt 4: 100 * 8 = 800ms
        assert_eq!(policy.calculate_max_delay(4), Duration::from_millis(800));
        assert_eq!(policy.compute_delay(4), Duration::from_millis(800));

        // Attempt 5: 100 * 16 = 1600ms, capped at 1000ms
        assert_eq!(policy.calculate_max_delay(5), Duration::from_millis(1000));
        assert_eq!(policy.compute_delay(5), Duration::from_millis(1000));
    }

    #[test]
    fn test_full_jitter_bounds_and_ratio() {
        let policy = BackoffPolicy::new(
            Duration::from_millis(100),
            Duration::from_millis(1000),
            2.0,
            true,
        );

        // Ratio testing
        assert_eq!(
            policy.compute_delay_with_ratio(1, 0.0),
            Duration::from_millis(0)
        );
        assert_eq!(
            policy.compute_delay_with_ratio(1, 0.5),
            Duration::from_millis(50)
        );
        assert_eq!(
            policy.compute_delay_with_ratio(1, 1.0),
            Duration::from_millis(100)
        );

        assert_eq!(
            policy.compute_delay_with_ratio(2, 0.5),
            Duration::from_millis(100)
        );

        // Random jitter bounds testing
        for _ in 0..50 {
            let delay1 = policy.compute_delay(1);
            assert!(delay1 <= Duration::from_millis(100));

            let delay2 = policy.compute_delay(2);
            assert!(delay2 <= Duration::from_millis(200));

            let delay3 = policy.compute_delay(3);
            assert!(delay3 <= Duration::from_millis(400));
        }
    }
}

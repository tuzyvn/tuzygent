use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use super::fault::{simulate_metrics_fault, MetricsError, MetricsFaultPolicy};
use super::types::{MetricsSnapshot, OodaMetrics, RemediationMetrics, TaskMetrics, TestMetrics};

#[derive(Clone)]
pub struct ResilientMetricsService {
    tasks_total: Arc<AtomicU64>,
    tasks_completed: Arc<AtomicU64>,
    tasks_failed: Arc<AtomicU64>,
    ooda_cycles_total: Arc<AtomicU64>,
    ooda_last_duration_micros: Arc<AtomicU64>,
    ooda_total_duration_micros: Arc<AtomicU64>,
    remediations_total: Arc<AtomicU64>,
    tests_total: Arc<AtomicU64>,
    tests_passed: Arc<AtomicU64>,
    tests_failed: Arc<AtomicU64>,
    cached_snapshot: Arc<RwLock<Option<String>>>,
    fault_policy: Arc<Mutex<MetricsFaultPolicy>>,
    max_retries: usize,
    backoff: Duration,
    max_capacity: usize,
    recovered_count: Arc<AtomicUsize>,
    retries_attempted: Arc<AtomicUsize>,
    panics_caught: Arc<AtomicUsize>,
}

impl Default for ResilientMetricsService {
    fn default() -> Self {
        Self {
            tasks_total: Arc::new(AtomicU64::new(0)),
            tasks_completed: Arc::new(AtomicU64::new(0)),
            tasks_failed: Arc::new(AtomicU64::new(0)),
            ooda_cycles_total: Arc::new(AtomicU64::new(0)),
            ooda_last_duration_micros: Arc::new(AtomicU64::new(0)),
            ooda_total_duration_micros: Arc::new(AtomicU64::new(0)),
            remediations_total: Arc::new(AtomicU64::new(0)),
            tests_total: Arc::new(AtomicU64::new(0)),
            tests_passed: Arc::new(AtomicU64::new(0)),
            tests_failed: Arc::new(AtomicU64::new(0)),
            cached_snapshot: Arc::new(RwLock::new(None)),
            fault_policy: Arc::new(Mutex::new(MetricsFaultPolicy::none())),
            max_retries: 3,
            backoff: Duration::from_millis(5),
            max_capacity: 10_000,
            recovered_count: Arc::new(AtomicUsize::new(0)),
            retries_attempted: Arc::new(AtomicUsize::new(0)),
            panics_caught: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl ResilientMetricsService {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_max_retries(mut self, max: usize) -> Self {
        self.max_retries = max;
        self
    }

    pub fn with_backoff(mut self, backoff: Duration) -> Self {
        self.backoff = backoff;
        self
    }

    pub fn with_max_capacity(mut self, cap: usize) -> Self {
        self.max_capacity = cap;
        self
    }

    pub fn with_fault_policy(mut self, policy: MetricsFaultPolicy) -> Self {
        self.fault_policy = Arc::new(Mutex::new(policy));
        self
    }

    pub fn fault_policy(&self) -> Arc<Mutex<MetricsFaultPolicy>> {
        self.fault_policy.clone()
    }

    pub fn record_task_submitted(&self) {
        self.tasks_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_task_completed(&self, _duration_secs: f64) {
        self.tasks_completed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_task_failed(&self, _duration_secs: f64) {
        self.tasks_failed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_ooda_cycle(&self, duration_secs: f64) {
        self.ooda_cycles_total.fetch_add(1, Ordering::Relaxed);
        let micros = (duration_secs * 1_000_000.0).max(0.0) as u64;
        self.ooda_last_duration_micros
            .store(micros, Ordering::Relaxed);
        self.ooda_total_duration_micros
            .fetch_add(micros, Ordering::Relaxed);
    }

    pub fn record_remediation(&self) {
        self.remediations_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_test_result(&self, success: bool) {
        self.tests_total.fetch_add(1, Ordering::Relaxed);
        if success {
            self.tests_passed.fetch_add(1, Ordering::Relaxed);
        } else {
            self.tests_failed.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        let total_tasks = self.tasks_total.load(Ordering::Relaxed);
        let completed_tasks = self.tasks_completed.load(Ordering::Relaxed);
        let failed_tasks = self.tasks_failed.load(Ordering::Relaxed);

        let ooda_cycles = self.ooda_cycles_total.load(Ordering::Relaxed);
        let last_micros = self.ooda_last_duration_micros.load(Ordering::Relaxed);
        let total_micros = self.ooda_total_duration_micros.load(Ordering::Relaxed);

        let remediations = self.remediations_total.load(Ordering::Relaxed);

        let total_tests = self.tests_total.load(Ordering::Relaxed);
        let passed_tests = self.tests_passed.load(Ordering::Relaxed);
        let failed_tests = self.tests_failed.load(Ordering::Relaxed);

        MetricsSnapshot {
            timestamp: chrono::Utc::now(),
            tasks: TaskMetrics {
                total: total_tasks,
                completed: completed_tasks,
                failed: failed_tasks,
            },
            ooda: OodaMetrics {
                total_cycles: ooda_cycles,
                last_duration_seconds: (last_micros as f64) / 1_000_000.0,
                total_duration_seconds: (total_micros as f64) / 1_000_000.0,
            },
            remediations: RemediationMetrics {
                total_remediations: remediations,
                successful_remediations: remediations,
            },
            tests: TestMetrics {
                total_tests,
                passed_tests,
                failed_tests,
            },
            custom_gauges: Vec::new(),
            custom_counters: Vec::new(),
        }
    }

    pub fn render_prometheus_text(&self) -> Result<String, MetricsError> {
        let mut attempts = 0;
        let mut had_transient_failure = false;

        loop {
            let fault = if let Ok(mut policy) = self.fault_policy.lock() {
                policy.should_inject()
            } else {
                None
            };

            let result = if let Some(f) = fault {
                Err(simulate_metrics_fault(f))
            } else {
                let snap = self.snapshot();
                Ok(snap.to_prometheus_text())
            };

            match result {
                Ok(rendered) => {
                    if had_transient_failure {
                        self.recovered_count.fetch_add(1, Ordering::Relaxed);
                    }
                    if let Ok(mut cache) = self.cached_snapshot.write() {
                        *cache = Some(rendered.clone());
                    }
                    return Ok(rendered);
                }
                Err(err) if err.is_retryable() && attempts < self.max_retries => {
                    attempts += 1;
                    had_transient_failure = true;
                    self.retries_attempted.fetch_add(1, Ordering::Relaxed);
                    std::thread::sleep(self.backoff);
                }
                Err(err) => {
                    // Try fallback to cached snapshot
                    if let Ok(cache) = self.cached_snapshot.read() {
                        if let Some(cached) = cache.as_ref() {
                            self.recovered_count.fetch_add(1, Ordering::Relaxed);
                            return Ok(cached.clone());
                        }
                    }

                    if attempts >= self.max_retries && err.is_retryable() {
                        return Err(MetricsError::MaxRetriesExceeded(self.max_retries));
                    }
                    return Err(err);
                }
            }
        }
    }

    pub fn recovered_count(&self) -> usize {
        self.recovered_count.load(Ordering::Relaxed)
    }

    pub fn retries_attempted(&self) -> usize {
        self.retries_attempted.load(Ordering::Relaxed)
    }

    pub fn panics_caught_count(&self) -> usize {
        self.panics_caught.load(Ordering::Relaxed)
    }

    pub fn max_capacity(&self) -> usize {
        self.max_capacity
    }
}

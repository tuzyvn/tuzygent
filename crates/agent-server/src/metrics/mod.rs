pub mod benchmark;
pub mod fault;
pub mod handler;
pub mod resilience;
pub mod types;

pub use benchmark::{MetricsBenchmarkHarness, MetricsBenchmarkReport};
pub use fault::{
    simulate_metrics_fault, MetricsError, MetricsErrorTier, MetricsFault, MetricsFaultPolicy,
};
pub use handler::metrics_handler;
pub use resilience::ResilientMetricsService;
pub use types::{
    MetricType, MetricsSnapshot, OodaMetrics, RemediationMetrics, TaskMetrics, TestMetrics,
};

#[cfg(test)]
mod public_types_api_tests {
    use super::types::{
        MetricType, MetricsSnapshot, OodaMetrics, RemediationMetrics, TaskMetrics, TestMetrics,
    };

    #[test]
    fn types_module_exposes_all_public_metric_types() {
        let _: MetricType = MetricType::Counter;
        let _: MetricsSnapshot = MetricsSnapshot::default();
        let _: OodaMetrics = OodaMetrics::default();
        let _: RemediationMetrics = RemediationMetrics::default();
        let _: TaskMetrics = TaskMetrics::default();
        let _: TestMetrics = TestMetrics::default();
    }
}

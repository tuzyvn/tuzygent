use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{OodaMetrics, RemediationMetrics, TaskMetrics, TestMetrics};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct MetricsSnapshot {
    pub timestamp: DateTime<Utc>,
    pub tasks: TaskMetrics,
    pub ooda: OodaMetrics,
    pub remediations: RemediationMetrics,
    pub tests: TestMetrics,
    pub custom_gauges: Vec<(String, f64)>,
    pub custom_counters: Vec<(String, u64)>,
}

impl Default for MetricsSnapshot {
    fn default() -> Self {
        Self {
            timestamp: Utc::now(),
            tasks: TaskMetrics::default(),
            ooda: OodaMetrics::default(),
            remediations: RemediationMetrics::default(),
            tests: TestMetrics::default(),
            custom_gauges: Vec::new(),
            custom_counters: Vec::new(),
        }
    }
}

impl MetricsSnapshot {
    pub fn to_prometheus_text(&self) -> String {
        let mut out = String::with_capacity(1024);

        // agent_tasks_total
        out.push_str(
            "# HELP agent_tasks_total Total number of tasks processed by the agent server\n",
        );
        out.push_str("# TYPE agent_tasks_total counter\n");
        out.push_str(&format!(
            "agent_tasks_total{{status=\"completed\"}} {}\n",
            self.tasks.completed
        ));
        out.push_str(&format!(
            "agent_tasks_total{{status=\"failed\"}} {}\n",
            self.tasks.failed
        ));
        out.push_str(&format!("agent_tasks_total {}\n\n", self.tasks.total));

        // agent_ooda_duration_seconds
        out.push_str("# HELP agent_ooda_duration_seconds Duration of OODA loop execution cycles in seconds\n");
        out.push_str("# TYPE agent_ooda_duration_seconds gauge\n");
        out.push_str(&format!(
            "agent_ooda_duration_seconds {:.6}\n\n",
            self.ooda.last_duration_seconds
        ));

        // agent_ooda_cycles_total
        out.push_str("# HELP agent_ooda_cycles_total Total count of executed OODA cycles\n");
        out.push_str("# TYPE agent_ooda_cycles_total counter\n");
        out.push_str(&format!(
            "agent_ooda_cycles_total {}\n\n",
            self.ooda.total_cycles
        ));

        // agent_remediations_total
        out.push_str(
            "# HELP agent_remediations_total Total number of remediation subtasks triggered\n",
        );
        out.push_str("# TYPE agent_remediations_total counter\n");
        out.push_str(&format!(
            "agent_remediations_total {}\n\n",
            self.remediations.total_remediations
        ));

        // agent_tests_success_ratio
        out.push_str("# HELP agent_tests_success_ratio Ratio of passing verification tests\n");
        out.push_str("# TYPE agent_tests_success_ratio gauge\n");
        out.push_str(&format!(
            "agent_tests_success_ratio {:.4}\n\n",
            self.tests.success_ratio()
        ));

        // Custom metrics
        for (name, val) in &self.custom_counters {
            out.push_str(&format!("{name} {val}\n"));
        }
        for (name, val) in &self.custom_gauges {
            out.push_str(&format!("{name} {:.6}\n", val));
        }

        out
    }
}

#[cfg(test)]
mod serialization_tests {
    use super::super::{OodaMetrics, RemediationMetrics, TaskMetrics, TestMetrics};
    use super::MetricsSnapshot;
    use chrono::{TimeZone, Utc};

    #[test]
    fn metrics_snapshot_serialization_is_compatible() -> Result<(), Box<dyn std::error::Error>> {
        let snapshot = MetricsSnapshot {
            timestamp: Utc
                .with_ymd_and_hms(2026, 11, 7, 12, 30, 0)
                .single()
                .ok_or_else(|| std::io::Error::other("invalid snapshot timestamp"))?,
            tasks: TaskMetrics {
                total: 12,
                completed: 9,
                failed: 3,
            },
            ooda: OodaMetrics {
                total_cycles: 4,
                last_duration_seconds: 1.25,
                total_duration_seconds: 7.5,
            },
            remediations: RemediationMetrics {
                total_remediations: 6,
                successful_remediations: 5,
            },
            tests: TestMetrics {
                total_tests: 20,
                passed_tests: 18,
                failed_tests: 2,
            },
            custom_gauges: vec![("queue_depth".to_owned(), 2.5)],
            custom_counters: vec![("requests_total".to_owned(), 17)],
        };
        let value = serde_json::to_value(&snapshot)?;
        assert_eq!(
            value,
            serde_json::json!({
                "timestamp": "2026-11-07T12:30:00Z",
                "tasks": {"total": 12, "completed": 9, "failed": 3},
                "ooda": {
                    "total_cycles": 4,
                    "last_duration_seconds": 1.25,
                    "total_duration_seconds": 7.5
                },
                "remediations": {
                    "total_remediations": 6,
                    "successful_remediations": 5
                },
                "tests": {"total_tests": 20, "passed_tests": 18, "failed_tests": 2},
                "custom_gauges": [["queue_depth", 2.5]],
                "custom_counters": [["requests_total", 17]]
            })
        );
        assert_eq!(serde_json::from_value::<MetricsSnapshot>(value)?, snapshot);
        Ok(())
    }
}

#[cfg(test)]
mod public_api_tests {
    use super::super::{OodaMetrics, TestMetrics};
    use super::MetricsSnapshot;

    #[test]
    fn metrics_snapshot_renders_standard_and_custom_prometheus_metrics() {
        let snapshot = MetricsSnapshot {
            tasks: super::super::TaskMetrics {
                total: 12,
                completed: 9,
                failed: 3,
            },
            ooda: OodaMetrics {
                total_cycles: 4,
                last_duration_seconds: 1.25,
                total_duration_seconds: 7.5,
            },
            remediations: super::super::RemediationMetrics {
                total_remediations: 6,
                successful_remediations: 5,
            },
            tests: TestMetrics {
                total_tests: 20,
                passed_tests: 18,
                failed_tests: 2,
            },
            custom_counters: vec![("requests_total".to_owned(), 17)],
            custom_gauges: vec![("queue_depth".to_owned(), 2.5)],
            ..MetricsSnapshot::default()
        };

        let rendered = snapshot.to_prometheus_text();

        for expected in [
            "# TYPE agent_tasks_total counter\n",
            "agent_tasks_total{status=\"completed\"} 9\n",
            "agent_tasks_total{status=\"failed\"} 3\n",
            "agent_tasks_total 12\n\n",
            "agent_ooda_duration_seconds 1.250000\n\n",
            "agent_ooda_cycles_total 4\n\n",
            "agent_remediations_total 6\n\n",
            "agent_tests_success_ratio 0.9000\n\n",
            "requests_total 17\n",
            "queue_depth 2.500000\n",
        ] {
            assert!(
                rendered.contains(expected),
                "missing {expected:?} in output: {rendered}"
            );
        }

        let counter_position = rendered.find("requests_total 17");
        let gauge_position = rendered.find("queue_depth 2.500000");
        assert!(
            matches!(
                (counter_position, gauge_position),
                (Some(counter), Some(gauge)) if counter < gauge
            ),
            "expected custom counter to appear before custom gauge; counter: {counter_position:?}, gauge: {gauge_position:?}"
        );
    }

    #[test]
    fn prometheus_output_preserves_help_types_formatting_and_custom_metric_order() {
        let snapshot = MetricsSnapshot {
            tasks: super::super::TaskMetrics {
                total: 12,
                completed: 9,
                failed: 3,
            },
            ooda: OodaMetrics {
                total_cycles: 4,
                last_duration_seconds: 1.25,
                total_duration_seconds: 7.5,
            },
            remediations: super::super::RemediationMetrics {
                total_remediations: 6,
                successful_remediations: 5,
            },
            tests: TestMetrics {
                total_tests: 20,
                passed_tests: 18,
                failed_tests: 2,
            },
            custom_counters: vec![("requests_total".to_owned(), 17)],
            custom_gauges: vec![("queue_depth".to_owned(), 2.5)],
            ..MetricsSnapshot::default()
        };

        assert_eq!(
            snapshot.to_prometheus_text(),
            concat!(
                "# HELP agent_tasks_total Total number of tasks processed by the agent server\n",
                "# TYPE agent_tasks_total counter\n",
                "agent_tasks_total{status=\"completed\"} 9\n",
                "agent_tasks_total{status=\"failed\"} 3\n",
                "agent_tasks_total 12\n\n",
                "# HELP agent_ooda_duration_seconds Duration of OODA loop execution cycles in seconds\n",
                "# TYPE agent_ooda_duration_seconds gauge\n",
                "agent_ooda_duration_seconds 1.250000\n\n",
                "# HELP agent_ooda_cycles_total Total count of executed OODA cycles\n",
                "# TYPE agent_ooda_cycles_total counter\n",
                "agent_ooda_cycles_total 4\n\n",
                "# HELP agent_remediations_total Total number of remediation subtasks triggered\n",
                "# TYPE agent_remediations_total counter\n",
                "agent_remediations_total 6\n\n",
                "# HELP agent_tests_success_ratio Ratio of passing verification tests\n",
                "# TYPE agent_tests_success_ratio gauge\n",
                "agent_tests_success_ratio 0.9000\n\n",
                "requests_total 17\n",
                "queue_depth 2.500000\n",
            )
        );
    }

    #[test]
    fn metrics_snapshot_without_custom_metrics_handles_missing_positions_safely() {
        let rendered = MetricsSnapshot::default().to_prometheus_text();

        let counter_position = rendered.find("requests_total 17");
        let gauge_position = rendered.find("queue_depth 2.500000");

        assert_eq!(counter_position, None);
        assert_eq!(gauge_position, None);
    }
}

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum MetricType {
    Counter,
    Gauge,
    Histogram,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct TaskMetrics {
    pub total: u64,
    pub completed: u64,
    pub failed: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct OodaMetrics {
    pub total_cycles: u64,
    pub last_duration_seconds: f64,
    pub total_duration_seconds: f64,
}

impl OodaMetrics {
    pub fn average_duration_seconds(&self) -> f64 {
        if self.total_cycles == 0 {
            0.0
        } else {
            self.total_duration_seconds / (self.total_cycles as f64)
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct RemediationMetrics {
    pub total_remediations: u64,
    pub successful_remediations: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct TestMetrics {
    pub total_tests: u64,
    pub passed_tests: u64,
    pub failed_tests: u64,
}

impl TestMetrics {
    pub fn success_ratio(&self) -> f64 {
        if self.total_tests == 0 {
            1.0
        } else {
            (self.passed_tests as f64) / (self.total_tests as f64)
        }
    }
}

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
    use super::{
        MetricType, MetricsSnapshot, OodaMetrics, RemediationMetrics, TaskMetrics, TestMetrics,
    };
    use chrono::{TimeZone, Utc};

    #[test]
    fn metric_type_serialization_is_compatible() -> Result<(), Box<dyn std::error::Error>> {
        for (metric_type, expected) in [
            (MetricType::Counter, "Counter"),
            (MetricType::Gauge, "Gauge"),
            (MetricType::Histogram, "Histogram"),
        ] {
            let value = serde_json::to_value(metric_type)?;
            assert_eq!(value, expected);
            assert_eq!(serde_json::from_value::<MetricType>(value)?, metric_type);
        }
        Ok(())
    }

    #[test]
    fn invalid_metric_type_is_rejected_without_panicking() {
        let result = serde_json::from_value::<MetricType>(serde_json::json!("Summary"));

        assert!(result.is_err(), "unknown metric types should be rejected");
    }

    #[test]
    fn task_metrics_serialization_is_compatible() -> Result<(), Box<dyn std::error::Error>> {
        let metrics = TaskMetrics {
            total: 12,
            completed: 9,
            failed: 3,
        };
        let value = serde_json::to_value(&metrics)?;
        assert_eq!(
            value,
            serde_json::json!({"total": 12, "completed": 9, "failed": 3})
        );
        assert_eq!(serde_json::from_value::<TaskMetrics>(value)?, metrics);
        Ok(())
    }

    #[test]
    fn ooda_metrics_serialization_is_compatible() -> Result<(), Box<dyn std::error::Error>> {
        let metrics = OodaMetrics {
            total_cycles: 4,
            last_duration_seconds: 1.25,
            total_duration_seconds: 7.5,
        };
        let value = serde_json::to_value(&metrics)?;
        assert_eq!(
            value,
            serde_json::json!({
                "total_cycles": 4,
                "last_duration_seconds": 1.25,
                "total_duration_seconds": 7.5
            })
        );
        assert_eq!(serde_json::from_value::<OodaMetrics>(value)?, metrics);
        Ok(())
    }

    #[test]
    fn remediation_metrics_serialization_is_compatible() -> Result<(), Box<dyn std::error::Error>> {
        let metrics = RemediationMetrics {
            total_remediations: 6,
            successful_remediations: 5,
        };
        let value = serde_json::to_value(&metrics)?;
        assert_eq!(
            value,
            serde_json::json!({
                "total_remediations": 6,
                "successful_remediations": 5
            })
        );
        assert_eq!(
            serde_json::from_value::<RemediationMetrics>(value)?,
            metrics
        );
        Ok(())
    }

    #[test]
    fn test_metrics_serialization_is_compatible() -> Result<(), Box<dyn std::error::Error>> {
        let metrics = TestMetrics {
            total_tests: 20,
            passed_tests: 18,
            failed_tests: 2,
        };
        let value = serde_json::to_value(&metrics)?;
        assert_eq!(
            value,
            serde_json::json!({"total_tests": 20, "passed_tests": 18, "failed_tests": 2})
        );
        assert_eq!(serde_json::from_value::<TestMetrics>(value)?, metrics);
        Ok(())
    }

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

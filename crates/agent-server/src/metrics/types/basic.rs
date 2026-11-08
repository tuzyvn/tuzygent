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

#[cfg(test)]
mod serialization_tests {
    use super::{MetricType, OodaMetrics, RemediationMetrics, TaskMetrics, TestMetrics};
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
}

#[cfg(test)]
mod public_api_tests {
    use super::{OodaMetrics, TestMetrics};

    #[test]
    fn ooda_average_duration_is_zero_without_cycles_and_averages_recorded_duration() {
        assert_eq!(OodaMetrics::default().average_duration_seconds(), 0.0);

        let metrics = OodaMetrics {
            total_cycles: 4,
            total_duration_seconds: 7.5,
            ..OodaMetrics::default()
        };
        assert_eq!(metrics.average_duration_seconds(), 1.875);
    }

    #[test]
    fn test_success_ratio_is_one_without_tests_and_uses_passed_count_otherwise() {
        assert_eq!(TestMetrics::default().success_ratio(), 1.0);

        let metrics = TestMetrics {
            total_tests: 20,
            passed_tests: 18,
            failed_tests: 2,
        };
        assert_eq!(metrics.success_ratio(), 0.9);
    }
}

use axum::Json;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

pub async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        timestamp: chrono::Utc::now(),
    })
}

#[cfg(test)]
mod tests {
    use super::HealthResponse;
    use chrono::{TimeZone, Utc};
    use serde::Deserialize;

    fn fixed_timestamp(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
    ) -> Result<chrono::DateTime<Utc>, &'static str> {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, second)
            .single()
            .ok_or("invalid UTC timestamp components")
    }

    #[test]
    fn invalid_health_timestamp_returns_error_instead_of_panicking() {
        let result = fixed_timestamp(2026, 2, 30, 12, 34, 56);

        assert_eq!(result, Err("invalid UTC timestamp components"));
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct DeserializedHealthResponse {
        status: String,
        version: String,
        timestamp: chrono::DateTime<Utc>,
    }

    #[test]
    fn health_response_serialization_is_compatible() -> Result<(), Box<dyn std::error::Error>> {
        let timestamp = fixed_timestamp(2026, 11, 7, 12, 34, 56)?;
        let response = HealthResponse {
            status: "ok",
            version: "1.2.3",
            timestamp,
        };

        let value = serde_json::to_value(&response)?;
        assert_eq!(
            value,
            serde_json::json!({
                "status": "ok",
                "version": "1.2.3",
                "timestamp": "2026-11-07T12:34:56Z"
            })
        );

        let deserialized: DeserializedHealthResponse = serde_json::from_value(value)?;
        assert_eq!(
            deserialized,
            DeserializedHealthResponse {
                status: "ok".to_owned(),
                version: "1.2.3".to_owned(),
                timestamp,
            }
        );
        Ok(())
    }
}

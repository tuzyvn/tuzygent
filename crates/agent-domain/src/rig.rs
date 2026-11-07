use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RigMetadata {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RigOperatorConfig {
    pub agent_id: String,
    pub name: Option<String>,
    pub role: Option<String>,
    pub persona_path: Option<String>,
    pub engine: Option<String>,
    pub model: Option<String>,
}

fn default_harness() -> String {
    "process".to_string()
}

fn default_autonomy() -> String {
    "full".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RigSeatConfig {
    pub seat_id: String,
    pub name: Option<String>,
    pub role: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub persona_path: Option<String>,
    pub engine: Option<String>,
    pub model: Option<String>,
    #[serde(default = "default_harness")]
    pub harness: String, // "tmux", "process", "pty", "mock"
    pub voice_preset: Option<String>,
    #[serde(default = "default_autonomy")]
    pub autonomy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RigChannelConfig {
    pub from_seat: String,
    pub to_seat: String,
    #[serde(default)]
    pub mode: Option<String>, // "direct", "broadcast", "review"
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RigSpec {
    pub rig: RigMetadata,
    pub operator: RigOperatorConfig,
    pub seats: Vec<RigSeatConfig>,
    #[serde(default)]
    pub channels: Vec<RigChannelConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SeatSnapshot {
    pub seat_id: String,
    pub name: String,
    pub role: String,
    pub harness: String,
    pub is_active: bool,
    pub terminal_buffer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RigSnapshot {
    pub snapshot_id: String,
    pub rig_id: String,
    pub rig_name: String,
    pub created_at: String,
    pub seats: Vec<SeatSnapshot>,
    pub spec_yaml: String,
}

impl RigSnapshot {
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(json_str: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json_str)
    }
}

impl RigSpec {
    pub fn from_yaml_str(yaml_str: &str) -> Result<Self, serde_yaml_ng::Error> {
        serde_yaml_ng::from_str(yaml_str)
    }

    pub fn to_yaml_str(&self) -> Result<String, serde_yaml_ng::Error> {
        serde_yaml_ng::to_string(self)
    }

    pub fn find_seat(&self, seat_id: &str) -> Option<&RigSeatConfig> {
        self.seats.iter().find(|s| s.seat_id == seat_id)
    }

    pub fn find_seats_by_capability(&self, capability: &str) -> Vec<&RigSeatConfig> {
        let cap_lower = capability.to_lowercase();
        self.seats
            .iter()
            .filter(|s| {
                s.capabilities
                    .iter()
                    .any(|c| c.to_lowercase().contains(&cap_lower))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rig_spec_yaml_roundtrip() {
        let sample_yaml = r#"
rig:
  id: "software-dev"
  name: "Software Engineering Rig"
  description: "Autonomous software development team with lead, backend and QA"
  version: "1.0.0"

operator:
  agent_id: "agent-lead"
  name: "Alex Tech Lead"
  role: "Lead Architect"
  engine: "antigravity"
  model: "gemini-3.8-flash-high"

seats:
  - seat_id: "backend-dev"
    name: "Tèo Backend"
    role: "Senior Rust Developer"
    capabilities:
      - "rust compilation"
      - "api design"
      - "sql migrations"
    engine: "claude-code"
    harness: "tmux"
    voice_preset: "cinematic_warm"

  - seat_id: "qa-tester"
    name: "Mai QA"
    role: "Quality Assurance & Security"
    capabilities:
      - "integration testing"
      - "security auditing"
    engine: "local"
    harness: "process"

channels:
  - from_seat: "backend-dev"
    to_seat: "qa-tester"
    mode: "review"
"#;

        let spec = RigSpec::from_yaml_str(sample_yaml).expect("Must parse valid YAML RigSpec");
        assert_eq!(spec.rig.id, "software-dev");
        assert_eq!(spec.operator.agent_id, "agent-lead");
        assert_eq!(spec.seats.len(), 2);
        assert_eq!(spec.channels.len(), 1);

        let qa_seats = spec.find_seats_by_capability("security");
        assert_eq!(qa_seats.len(), 1);
        assert_eq!(qa_seats[0].seat_id, "qa-tester");

        let serialized = spec.to_yaml_str().expect("Must re-serialize to YAML");
        let roundtrip_spec = RigSpec::from_yaml_str(&serialized).expect("Must parse roundtrip");
        assert_eq!(spec, roundtrip_spec);
    }

    #[test]
    fn test_seat_snapshot_json_roundtrip() {
        let snapshot = SeatSnapshot {
            seat_id: "backend-dev".to_string(),
            name: "Backend".to_string(),
            role: "Developer".to_string(),
            harness: "process".to_string(),
            is_active: true,
            terminal_buffer: Some("ready\n".to_string()),
        };
        let expected_json = serde_json::json!({
            "seat_id": "backend-dev",
            "name": "Backend",
            "role": "Developer",
            "harness": "process",
            "is_active": true,
            "terminal_buffer": "ready\n"
        });

        let serialized = serde_json::to_value(&snapshot).expect("Must serialize SeatSnapshot");
        assert_eq!(serialized, expected_json);

        let deserialized: SeatSnapshot =
            serde_json::from_value(expected_json).expect("Must deserialize SeatSnapshot JSON");
        assert_eq!(deserialized, snapshot);
    }

    #[test]
    fn test_rig_snapshot_json_roundtrip() {
        let snapshot = RigSnapshot {
            snapshot_id: "snap_software-dev_20261107_120000".to_string(),
            rig_id: "software-dev".to_string(),
            rig_name: "Software Engineering Rig".to_string(),
            created_at: "2026-11-07T12:00:00Z".to_string(),
            seats: vec![SeatSnapshot {
                seat_id: "backend-dev".to_string(),
                name: "Backend".to_string(),
                role: "Developer".to_string(),
                harness: "process".to_string(),
                is_active: true,
                terminal_buffer: None,
            }],
            spec_yaml: "rig:\n  id: software-dev\n".to_string(),
        };
        let expected_json = serde_json::json!({
            "snapshot_id": "snap_software-dev_20261107_120000",
            "rig_id": "software-dev",
            "rig_name": "Software Engineering Rig",
            "created_at": "2026-11-07T12:00:00Z",
            "seats": [{
                "seat_id": "backend-dev",
                "name": "Backend",
                "role": "Developer",
                "harness": "process",
                "is_active": true,
                "terminal_buffer": null
            }],
            "spec_yaml": "rig:\n  id: software-dev\n"
        });

        let serialized = serde_json::to_value(&snapshot).expect("Must serialize RigSnapshot");
        assert_eq!(serialized, expected_json);

        let serialized = serde_json::to_string(&expected_json).expect("Must encode JSON fixture");
        let deserialized = RigSnapshot::from_json(&serialized).expect("Must deserialize RigSnapshot");
        assert_eq!(deserialized, snapshot);
    }
}

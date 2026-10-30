use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub String);

impl AgentId {
    pub fn new(prefix: &str) -> Self {
        Self(format!("{}-{}", prefix, Uuid::new_v4().simple()))
    }

    pub fn from_str(id: &str) -> Self {
        Self(id.to_string())
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AutonomyLevel {
    Full,
    Supervised,
    ShadowMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentIdentity {
    pub id: AgentId,
    pub name: String,
    pub description: String,
    pub autonomy_level: AutonomyLevel,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRole {
    pub role_code: String,
    pub title: String,
    pub primary_domain: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum VoiceEngineType {
    #[serde(rename = "vieneu")]
    VieNeu,
    #[serde(rename = "cosyvoice")]
    CosyVoice,
    #[serde(rename = "omnivoice")]
    OmniVoice,
    #[serde(rename = "edge")]
    EdgeNeural,
    #[serde(rename = "sherpa_onnx")]
    SherpaOnnx,
    #[serde(untagged)]
    Custom(String),
}

impl Default for VoiceEngineType {
    fn default() -> Self {
        Self::VieNeu
    }
}

fn default_voice_id() -> String {
    "default_voice".to_string()
}

fn default_language() -> String {
    "vi".to_string()
}

fn default_speed() -> f32 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VoiceProfile {
    #[serde(default = "default_voice_id")]
    pub voice_id: String,
    #[serde(default)]
    pub engine: VoiceEngineType,
    #[serde(default)]
    pub voice_name: Option<String>,
    #[serde(default)]
    pub ref_audio_path: Option<String>,
    #[serde(default)]
    pub fixed_seed: Option<u64>,
    #[serde(default)]
    pub dsp_preset: Option<String>,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default = "default_speed")]
    pub speed: f32,
    #[serde(default)]
    pub pitch_shift_semitones: f32,
}

impl Default for VoiceProfile {
    fn default() -> Self {
        Self {
            voice_id: default_voice_id(),
            engine: VoiceEngineType::default(),
            voice_name: Some("Đức Trí".to_string()),
            ref_audio_path: None,
            fixed_seed: None,
            dsp_preset: None,
            language: default_language(),
            speed: 1.0,
            pitch_shift_semitones: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSoul {
    pub persona: String,
    pub tone_of_voice: String,
    pub core_values: Vec<String>,
    #[serde(default)]
    pub voice_profile: Option<VoiceProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMission {
    pub high_level_objective: String,
    pub kpis: Vec<String>,
    pub constraints: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPersonaProfile {
    pub identity: AgentIdentity,
    pub role: AgentRole,
    pub soul: AgentSoul,
    pub mission: AgentMission,
}

impl AgentPersonaProfile {
    pub fn build_system_prompt(&self) -> String {
        let mut prompt = String::new();
        prompt.push_str(&format!("# IDENTITY & PROFILE: {}\n", self.identity.name));
        prompt.push_str(&format!("- ID: {}\n", self.identity.id));
        prompt.push_str(&format!("- Title: {}\n", self.role.title));
        prompt.push_str(&format!("- Domain: {}\n", self.role.primary_domain));
        prompt.push_str(&format!("- Autonomy: {:?}\n\n", self.identity.autonomy_level));

        prompt.push_str("# PERSONA & SOUL (TÍNH CÁCH & PHONG THÁI)\n");
        prompt.push_str(&format!("- Persona: {}\n", self.soul.persona));
        prompt.push_str(&format!("- Tone of Voice: {}\n", self.soul.tone_of_voice));
        if let Some(ref vp) = self.soul.voice_profile {
            prompt.push_str(&format!("- Physical Voice Identity: {:?} (ID: {})\n", vp.engine, vp.voice_id));
            if let Some(ref timbre) = vp.voice_name {
                prompt.push_str(&format!("- Acoustic Timbre: {}\n", timbre));
            }
            if let Some(ref preset) = vp.dsp_preset {
                prompt.push_str(&format!("- Vocal DSP Preset: {}\n", preset));
            }
        }
        prompt.push_str("- Core Values:\n");
        for cv in &self.soul.core_values {
            prompt.push_str(&format!("  * {}\n", cv));
        }
        prompt.push_str("\n");

        prompt.push_str("# MISSION & OBJECTIVES (MỤC TIÊU & NHIỆM VỤ)\n");
        prompt.push_str(&format!("- Objective: {}\n", self.mission.high_level_objective));
        prompt.push_str("- Capabilities:\n");
        for cap in &self.role.capabilities {
            prompt.push_str(&format!("  * {}\n", cap));
        }
        prompt.push_str("- Constraints & Red Lines:\n");
        for c in &self.mission.constraints {
            prompt.push_str(&format!("  * {}\n", c));
        }
        prompt
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backward_compatible_deserialization() {
        let legacy_json = r#"{
            "persona": "Nghiêm túc, kỷ luật",
            "tone_of_voice": "Dứt khoát",
            "core_values": ["Reliability"]
        }"#;

        let soul: AgentSoul = serde_json::from_str(legacy_json).expect("Should deserialize legacy JSON without voice_profile");
        assert_eq!(soul.persona, "Nghiêm túc, kỷ luật");
        assert_eq!(soul.voice_profile, None);
    }

    #[test]
    fn test_voice_profile_serialization_and_prompt() {
        let soul = AgentSoul {
            persona: "Chibi Idol".to_string(),
            tone_of_voice: "Ngọt ngào, đáng yêu".to_string(),
            core_values: vec!["Kawaii".to_string()],
            voice_profile: Some(VoiceProfile {
                voice_id: "vp_miku_01".to_string(),
                engine: VoiceEngineType::VieNeu,
                voice_name: Some("Trúc Ly".to_string()),
                ref_audio_path: Some("storage/characters/char_1/voice_raw.wav".to_string()),
                fixed_seed: Some(777),
                dsp_preset: Some("kawaii_warm".to_string()),
                language: "vi".to_string(),
                speed: 0.95,
                pitch_shift_semitones: 1.8,
            }),
        };

        let profile = AgentPersonaProfile {
            identity: AgentIdentity {
                id: AgentId::from_str("char-miku"),
                name: "Miku Virtual Idol".to_string(),
                description: "Ca sĩ ảo anime".to_string(),
                autonomy_level: AutonomyLevel::Full,
                created_at: chrono::Utc::now(),
            },
            role: AgentRole {
                role_code: "IDOL".to_string(),
                title: "Virtual Idol".to_string(),
                primary_domain: "Entertainment".to_string(),
                capabilities: vec!["Singing".to_string()],
            },
            soul,
            mission: AgentMission {
                high_level_objective: "Truyền cảm hứng âm nhạc".to_string(),
                kpis: vec![],
                constraints: vec![],
            },
        };

        let prompt = profile.build_system_prompt();
        assert!(prompt.contains("Physical Voice Identity: VieNeu (ID: vp_miku_01)"));
        assert!(prompt.contains("Acoustic Timbre: Trúc Ly"));
        assert!(prompt.contains("Vocal DSP Preset: kawaii_warm"));

        // Roundtrip JSON
        let serialized = serde_json::to_string(&profile).expect("Must serialize");
        let deserialized: AgentPersonaProfile = serde_json::from_str(&serialized).expect("Must deserialize");
        assert_eq!(deserialized.soul.voice_profile.unwrap().voice_id, "vp_miku_01");
    }
}

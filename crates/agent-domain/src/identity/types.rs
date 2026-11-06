use crate::identity::fault::{IdentityError, IdentityFault};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub String);

impl AgentId {
    pub fn new(prefix: &str) -> Self {
        Self(format!("{}-{}", prefix, Uuid::new_v4().simple()))
    }

    #[allow(clippy::should_implement_trait)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentIdentity {
    pub id: AgentId,
    pub name: String,
    pub description: String,
    pub autonomy_level: AutonomyLevel,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentRole {
    pub role_code: String,
    pub title: String,
    pub primary_domain: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum VoiceEngineType {
    #[serde(rename = "vieneu")]
    #[default]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentSoul {
    pub persona: String,
    pub tone_of_voice: String,
    pub core_values: Vec<String>,
    #[serde(default)]
    pub voice_profile: Option<VoiceProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentMission {
    pub high_level_objective: String,
    pub kpis: Vec<String>,
    pub constraints: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentPersonaProfile {
    pub identity: AgentIdentity,
    pub role: AgentRole,
    pub soul: AgentSoul,
    pub mission: AgentMission,
}

impl AgentPersonaProfile {
    pub fn validate(&self) -> Result<(), IdentityError> {
        if self.identity.id.0.trim().is_empty() {
            return Err(IdentityError::InvalidIdentity(
                "Agent ID cannot be empty".to_string(),
            ));
        }
        if self.identity.name.trim().is_empty() {
            return Err(IdentityError::InvalidIdentity(
                "Agent name cannot be empty".to_string(),
            ));
        }
        Ok(())
    }

    pub fn simulate_fault(&self, fault: &IdentityFault) -> Result<Self, IdentityError> {
        match fault {
            IdentityFault::SocketTimeout => Err(IdentityError::SocketTimeout),
            IdentityFault::ConnectionReset => Err(IdentityError::ConnectionReset),
            IdentityFault::TransientIo => Err(IdentityError::TransientIo(
                "Simulated transient I/O failure during identity resolution".to_string(),
            )),
            IdentityFault::CorruptedProfile => Err(IdentityError::CorruptedProfile(
                "Agent persona profile corruption detected".to_string(),
            )),
            IdentityFault::ServiceUnavailable => Err(IdentityError::ServiceUnavailable(
                "Identity service temporarily unavailable".to_string(),
            )),
        }
    }

    pub fn build_system_prompt(&self) -> String {
        let mut prompt = String::new();
        prompt.push_str(&format!("# IDENTITY & PROFILE: {}\n", self.identity.name));
        prompt.push_str(&format!("- ID: {}\n", self.identity.id));
        prompt.push_str(&format!("- Title: {}\n", self.role.title));
        prompt.push_str(&format!("- Domain: {}\n", self.role.primary_domain));
        prompt.push_str(&format!(
            "- Autonomy: {:?}\n\n",
            self.identity.autonomy_level
        ));

        prompt.push_str("# PERSONA & SOUL (TÍNH CÁCH & PHONG THÁI)\n");
        prompt.push_str(&format!("- Persona: {}\n", self.soul.persona));
        prompt.push_str(&format!("- Tone of Voice: {}\n", self.soul.tone_of_voice));
        if let Some(ref vp) = self.soul.voice_profile {
            prompt.push_str(&format!(
                "- Physical Voice Identity: {:?} (ID: {})\n",
                vp.engine, vp.voice_id
            ));
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
        prompt.push('\n');

        prompt.push_str("# MISSION & OBJECTIVES (MỤC TIÊU & NHIỆM VỤ)\n");
        prompt.push_str(&format!(
            "- Objective: {}\n",
            self.mission.high_level_objective
        ));
        prompt.push_str("- Capabilities:\n");
        for cap in &self.role.capabilities {
            prompt.push_str(&format!("  * {}\n", cap));
        }
        if !self.mission.kpis.is_empty() {
            prompt.push_str("- KPIs:\n");
            for kpi in &self.mission.kpis {
                prompt.push_str(&format!("  * {}\n", kpi));
            }
        }
        prompt.push_str("- Constraints & Red Lines:\n");
        for c in &self.mission.constraints {
            prompt.push_str(&format!("  * {}\n", c));
        }
        prompt
    }

    pub fn estimated_size_bytes(&self) -> usize {
        let base = std::mem::size_of::<Self>();
        let heap_id = self.identity.id.0.len() + self.identity.name.len() + self.identity.description.len();
        let heap_role = self.role.role_code.len() + self.role.title.len() + self.role.primary_domain.len()
            + self.role.capabilities.iter().map(|s| s.len()).sum::<usize>();
        let heap_soul = self.soul.persona.len() + self.soul.tone_of_voice.len()
            + self.soul.core_values.iter().map(|s| s.len()).sum::<usize>();
        let heap_mission = self.mission.high_level_objective.len()
            + self.mission.kpis.iter().map(|s| s.len()).sum::<usize>()
            + self.mission.constraints.iter().map(|s| s.len()).sum::<usize>();
        base + heap_id + heap_role + heap_soul + heap_mission
    }
}

/// Stable persona key, scoped to a project. Display names are never storage keys.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PersonaId(pub String);

impl From<&AgentPersonaProfile> for PersonaId {
    fn from(profile: &AgentPersonaProfile) -> Self {
        Self(profile.identity.id.0.clone())
    }
}

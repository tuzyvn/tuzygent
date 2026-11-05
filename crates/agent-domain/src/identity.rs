use crate::audit_ledger::{AuditEventType, AuditLedger};
use crate::guardrail::ActionSeverity;
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum IdentityError {
    #[error("Invalid identity structure: {0}")]
    InvalidIdentity(String),
    #[error("Simulated socket timeout during identity resolution")]
    SocketTimeout,
    #[error("Connection reset during identity resolution")]
    ConnectionReset,
    #[error("Transient I/O failure during identity operation: {0}")]
    TransientIo(String),
    #[error("Corrupted agent persona profile: {0}")]
    CorruptedProfile(String),
    #[error("Identity service temporarily unavailable: {0}")]
    ServiceUnavailable(String),
    #[error("Max retry attempts ({0}) exceeded during identity recovery")]
    MaxRetriesExceeded(usize),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IdentityFault {
    SocketTimeout,
    ConnectionReset,
    CorruptedProfile,
    TransientIo,
    ServiceUnavailable,
}

#[derive(Debug, Clone, Default)]
pub struct IdentityFaultPolicy {
    fault: Option<IdentityFault>,
    fail_interval: Option<u32>,
    max_injections: Option<usize>,
    invocation_count: u32,
    injected_count: usize,
}

impl IdentityFaultPolicy {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn always(fault: IdentityFault) -> Self {
        Self {
            fault: Some(fault),
            fail_interval: None,
            max_injections: None,
            invocation_count: 0,
            injected_count: 0,
        }
    }

    pub fn intermittent(fault: IdentityFault, interval: u32) -> Self {
        Self {
            fault: Some(fault),
            fail_interval: Some(interval.max(1)),
            max_injections: None,
            invocation_count: 0,
            injected_count: 0,
        }
    }

    pub fn with_max_injections(mut self, max: usize) -> Self {
        self.max_injections = Some(max);
        self
    }

    pub fn should_inject(&mut self) -> Option<IdentityFault> {
        let fault = self.fault.as_ref()?;

        if let Some(max) = self.max_injections {
            if self.injected_count >= max {
                return None;
            }
        }

        self.invocation_count = self.invocation_count.wrapping_add(1);

        let inject = match self.fail_interval {
            Some(interval) if interval > 0 => (self.invocation_count - 1).is_multiple_of(interval),
            Some(_) => true,
            None => true,
        };

        if inject {
            self.injected_count += 1;
            Some(fault.clone())
        } else {
            None
        }
    }

    pub fn reset(&mut self) {
        self.invocation_count = 0;
        self.injected_count = 0;
    }
}

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
}

#[derive(Debug, Clone)]
pub struct ResilientIdentityManager {
    fault_policy: IdentityFaultPolicy,
    max_retries: usize,
    resolved_count: usize,
    recovered_count: usize,
    rejected_count: usize,
}

impl ResilientIdentityManager {
    pub fn new(fault_policy: IdentityFaultPolicy, max_retries: usize) -> Self {
        Self {
            fault_policy,
            max_retries,
            resolved_count: 0,
            recovered_count: 0,
            rejected_count: 0,
        }
    }

    pub fn with_fault_policy(fault_policy: IdentityFaultPolicy) -> Self {
        Self::new(fault_policy, 3)
    }

    pub fn resolved_count(&self) -> usize {
        self.resolved_count
    }

    pub fn recovered_count(&self) -> usize {
        self.recovered_count
    }

    pub fn rejected_count(&self) -> usize {
        self.rejected_count
    }

    pub fn fault_policy(&self) -> &IdentityFaultPolicy {
        &self.fault_policy
    }

    pub fn fault_policy_mut(&mut self) -> &mut IdentityFaultPolicy {
        &mut self.fault_policy
    }

    pub fn max_retries(&self) -> usize {
        self.max_retries
    }

    pub fn reset_counts(&mut self) {
        self.resolved_count = 0;
        self.recovered_count = 0;
        self.rejected_count = 0;
    }

    pub fn resolve_identity(
        &mut self,
        profile: AgentPersonaProfile,
    ) -> Result<AgentPersonaProfile, IdentityError> {
        if let Err(err) = profile.validate() {
            self.rejected_count += 1;
            return Err(err);
        }

        let mut attempt = 0;
        loop {
            attempt += 1;
            if let Some(fault) = self.fault_policy.should_inject() {
                match fault {
                    IdentityFault::SocketTimeout
                    | IdentityFault::ConnectionReset
                    | IdentityFault::TransientIo
                    | IdentityFault::ServiceUnavailable => {
                        if attempt <= self.max_retries {
                            self.recovered_count += 1;
                            continue;
                        } else {
                            return match fault {
                                IdentityFault::SocketTimeout => Err(IdentityError::SocketTimeout),
                                IdentityFault::ConnectionReset => {
                                    Err(IdentityError::ConnectionReset)
                                }
                                IdentityFault::TransientIo => Err(IdentityError::TransientIo(
                                    "Simulated transient I/O failure during identity resolution"
                                        .to_string(),
                                )),
                                IdentityFault::ServiceUnavailable => {
                                    Err(IdentityError::ServiceUnavailable(
                                        "Identity service temporarily unavailable".to_string(),
                                    ))
                                }
                                _ => unreachable!(),
                            };
                        }
                    }
                    IdentityFault::CorruptedProfile => {
                        return Err(IdentityError::CorruptedProfile(
                            "Agent persona profile corruption detected".to_string(),
                        ));
                    }
                }
            } else {
                self.resolved_count += 1;
                return Ok(profile);
            }
        }
    }

    pub fn resolve_and_record(
        &mut self,
        ledger: &mut AuditLedger,
        profile: AgentPersonaProfile,
    ) -> Result<AgentPersonaProfile, IdentityError> {
        if let Err(err) = profile.validate() {
            self.rejected_count += 1;
            ledger.record_event(
                AuditEventType::CommandBlocked,
                ActionSeverity::Destructive,
                format!(
                    "Agent identity rejected for agent '{}': {}",
                    profile.identity.id, err
                ),
            );
            return Err(err);
        }

        let mut attempt = 0;
        loop {
            attempt += 1;
            if let Some(fault) = self.fault_policy.should_inject() {
                match fault {
                    IdentityFault::SocketTimeout
                    | IdentityFault::ConnectionReset
                    | IdentityFault::TransientIo
                    | IdentityFault::ServiceUnavailable => {
                        if attempt <= self.max_retries {
                            self.recovered_count += 1;
                            continue;
                        } else {
                            let err = match fault {
                                IdentityFault::SocketTimeout => IdentityError::SocketTimeout,
                                IdentityFault::ConnectionReset => {
                                    IdentityError::ConnectionReset
                                }
                                IdentityFault::TransientIo => IdentityError::TransientIo(
                                    "Simulated transient I/O failure during identity resolution"
                                        .to_string(),
                                ),
                                IdentityFault::ServiceUnavailable => {
                                    IdentityError::ServiceUnavailable(
                                        "Identity service temporarily unavailable".to_string(),
                                    )
                                }
                                _ => unreachable!(),
                            };
                            ledger.record_event(
                                AuditEventType::CommandException,
                                ActionSeverity::HighImpact,
                                format!(
                                    "Identity resolution exception for agent '{}': {}",
                                    profile.identity.id, err
                                ),
                            );
                            return Err(err);
                        }
                    }
                    IdentityFault::CorruptedProfile => {
                        let err = IdentityError::CorruptedProfile(
                            "Agent persona profile corruption detected".to_string(),
                        );
                        ledger.record_event(
                            AuditEventType::CommandException,
                            ActionSeverity::HighImpact,
                            format!(
                                "Identity resolution exception for agent '{}': {}",
                                profile.identity.id, err
                            ),
                        );
                        return Err(err);
                    }
                }
            } else {
                self.resolved_count += 1;
                ledger.record_event(
                    AuditEventType::CommandAllowed,
                    ActionSeverity::Routine,
                    format!("Identity resolved for agent '{}'", profile.identity.id),
                );
                return Ok(profile);
            }
        }
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

        let soul: AgentSoul = serde_json::from_str(legacy_json)
            .expect("Should deserialize legacy JSON without voice_profile");
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
        let deserialized: AgentPersonaProfile =
            serde_json::from_str(&serialized).expect("Must deserialize");
        assert_eq!(
            deserialized
                .soul
                .voice_profile
                .as_ref()
                .map(|vp| vp.voice_id.as_str()),
            Some("vp_miku_01")
        );
    }
}

/// Stable persona key, scoped to a project. Display names are never storage keys.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PersonaId(pub String);
impl From<&AgentPersonaProfile> for PersonaId {
    fn from(profile: &AgentPersonaProfile) -> Self { Self(profile.identity.id.0.clone()) }
}

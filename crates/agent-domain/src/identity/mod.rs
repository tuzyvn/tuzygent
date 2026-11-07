mod benchmark;
mod fault;
mod resilience;
mod types;

pub use benchmark::*;
pub use fault::*;
pub use resilience::*;
pub use types::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_voice_engine_type_json_compatibility() {
        let cases = [
            (VoiceEngineType::VieNeu, "vieneu"),
            (VoiceEngineType::CosyVoice, "cosyvoice"),
            (VoiceEngineType::OmniVoice, "omnivoice"),
            (VoiceEngineType::EdgeNeural, "edge"),
            (VoiceEngineType::SherpaOnnx, "sherpa_onnx"),
            (VoiceEngineType::Custom("local-engine".to_string()), "local-engine"),
        ];

        for (engine, expected) in cases {
            let expected_json = format!("\"{expected}\"");
            let serialized = serde_json::to_string(&engine).expect("engine type serializes");
            assert_eq!(serialized, expected_json);

            let restored: VoiceEngineType =
                serde_json::from_str(&expected_json).expect("current JSON fixture deserializes");
            assert_eq!(restored, engine);
            assert_eq!(
                serde_json::to_string(&restored).expect("restored engine type serializes"),
                expected_json
            );
        }
    }

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

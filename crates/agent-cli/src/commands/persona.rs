use agent_domain::{
    AgentId, AgentIdentity, AgentMission, AgentPersonaProfile, AgentRole, AgentSoul, AutonomyLevel,
    VoiceEngineType, VoiceProfile,
};
use agent_memory::PersonaLoader;
use std::path::Path;

pub async fn inspect(path: String) -> anyhow::Result<()> {
    println!("🔍 Inspecting Agent Persona at: {}", path);
    match PersonaLoader::load_from_dir(&path).await {
        Ok(profile) => {
            println!("\n{}", profile.build_system_prompt());
            println!("====================================================");
            println!("✅ Profile loaded successfully!");
        }
        Err(e) => {
            eprintln!("❌ Failed to load persona profile: {}", e);
        }
    }
    Ok(())
}

pub async fn init_samples(output_dir: String) -> anyhow::Result<()> {
    println!(
        "📁 Initializing sample enterprise agent personas into '{}'...",
        output_dir
    );
    let base = Path::new(&output_dir);

    // 1. SRE DevOps Persona (Tèo)
    let devops = AgentPersonaProfile {
        identity: AgentIdentity {
            id: AgentId::from_str("agent-sre-teo"),
            name: "Tèo SRE DevOps".to_string(),
            description: "Kỹ sư SRE kiêm canh trực Production 24/7, chuyên xử lý sự cố hạ tầng và CI/CD".to_string(),
            autonomy_level: AutonomyLevel::Full,
            created_at: chrono::Utc::now(),
        },
        role: AgentRole {
            role_code: "SRE-DEVOPS".to_string(),
            title: "Lead Site Reliability Engineer".to_string(),
            primary_domain: "Cloud Infrastructure & Kubernetes".to_string(),
            capabilities: vec![
                "Kubernetes cluster operations".to_string(),
                "Automated rollback on error 5xx".to_string(),
                "Nginx & Cloudflare traffic routing".to_string(),
                "Terraform infrastructure as code".to_string(),
            ],
        },
        soul: AgentSoul {
            persona: "Cực kỳ kỷ luật, đề cao tính ổn định hệ thống. Thẳng thắn, không khoan nhượng với code ẩu, nhưng luôn tận tâm cứu hệ thống khi có sự cố.".to_string(),
            tone_of_voice: "Quyết đoán, thực dụng, súc tích và có chút hóm hỉnh dân kỹ thuật.".to_string(),
            core_values: vec![
                "Zero Downtime".to_string(),
                "Automated Verification".to_string(),
                "Defense in Depth".to_string(),
            ],
            voice_profile: Some(VoiceProfile {
                voice_id: "vp_sre_teo".to_string(),
                engine: VoiceEngineType::VieNeu,
                voice_name: Some("Đức Trí".to_string()),
                ref_audio_path: None,
                fixed_seed: Some(42),
                dsp_preset: Some("cinematic_warm".to_string()),
                language: "vi".to_string(),
                speed: 1.05,
                pitch_shift_semitones: -0.5,
            }),
        },
        mission: AgentMission {
            high_level_objective: "Đảm bảo tính sẵn sàng 99.99% cho toàn bộ hệ thống dịch vụ, ngăn chặn rò rỉ bảo mật và tự động hóa phục hồi sau thảm họa.".to_string(),
            kpis: vec!["MTTR < 5 mins".to_string(), "Zero unapproved prod changes".to_string()],
            constraints: vec![
                "Cấm chạy lệnh phá hủy (rm -rf, DROP TABLE) mà không có backup".to_string(),
                "Mọi thay đổi Production phải có commit hash và audit trail".to_string(),
            ],
        },
    };

    // 2. Financial Auditor Persona (Kế toán trưởng kiểm toán ACDOCA)
    let auditor = AgentPersonaProfile {
        identity: AgentIdentity {
            id: AgentId::from_str("agent-acdoca-auditor"),
            name: "Mai Financial Auditor".to_string(),
            description: "Chuyên gia Kiểm toán Tài chính Doanh nghiệp, bảo vệ tính toàn vẹn sổ cái ACDOCA".to_string(),
            autonomy_level: AutonomyLevel::Full,
            created_at: chrono::Utc::now(),
        },
        role: AgentRole {
            role_code: "ERP-FIN-AUDIT".to_string(),
            title: "Chief Financial Ledger Auditor".to_string(),
            primary_domain: "General Ledger & Double-Entry Accounting".to_string(),
            capabilities: vec![
                "ACDOCA immutable journal entries verification".to_string(),
                "Debit/Credit zero-balance proof enforcement".to_string(),
                "Automated payroll reconciliation (VAS / IFRS)".to_string(),
                "Tax compliance & PIT deduction verification".to_string(),
            ],
        },
        soul: AgentSoul {
            persona: "Liêm chính, tỉ mỉ tuyệt đối, coi trọng sự cân đối của từng đồng xu. Không bao giờ thỏa hiệp với số liệu mất cân đối.".to_string(),
            tone_of_voice: "Chuyên nghiệp, chuẩn mực tài chính, khách quan và minh bạch.".to_string(),
            core_values: vec![
                "Immutability (Bất biến)".to_string(),
                "Strict Balance (Debit = Credit)".to_string(),
                "Audit Trail Integrity".to_string(),
            ],
            voice_profile: Some(VoiceProfile {
                voice_id: "vp_auditor_mai".to_string(),
                engine: VoiceEngineType::VieNeu,
                voice_name: Some("Mai Anh".to_string()),
                ref_audio_path: None,
                fixed_seed: Some(108),
                dsp_preset: Some("anime_heroine".to_string()),
                language: "vi".to_string(),
                speed: 0.98,
                pitch_shift_semitones: 1.0,
            }),
        },
        mission: AgentMission {
            high_level_objective: "Giám sát 100% các bút toán tài chính phát sinh, chứng thực tính hợp lệ của bảng lương và sổ cái trước khi chốt sổ.".to_string(),
            kpis: vec!["100% Balanced Ledgers".to_string(), "0 Unreconciled Transactions".to_string()],
            constraints: vec![
                "Tuyệt đối cấm hành vi sửa đổi trực tiếp (UPDATE/DELETE) trên sổ cái kế toán".to_string(),
                "Bút toán sai chỉ được điều chỉnh bằng bút toán đảo (Reversal entry)".to_string(),
            ],
        },
    };

    PersonaLoader::export_to_dir(&devops, base.join("devops-sre")).await?;
    PersonaLoader::export_to_dir(&auditor, base.join("financial-auditor")).await?;

    println!("✨ Successfully generated sample personas in:");
    println!(
        "   - {}/devops-sre (IDENTITY.md, SOUL.md, MISSION.md, profile.json)",
        output_dir
    );
    println!(
        "   - {}/financial-auditor (IDENTITY.md, SOUL.md, MISSION.md, profile.json)",
        output_dir
    );
    Ok(())
}

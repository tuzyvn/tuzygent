use agent_domain::{
    AgentId, AgentIdentity, AgentMission, AgentPersonaProfile, AgentRole, AgentSoul, AutonomyLevel,
};
use anyhow::{Context, Result};
use std::path::Path;
use tokio::fs;

pub struct PersonaLoader;

impl PersonaLoader {
    /// Nạp cấu hình Agent từ một thư mục chứa profile:
    /// - profile.yaml hoặc profile.json
    /// HOẶC
    /// - IDENTITY.md, SOUL.md, MISSION.md, ROLE.md
    pub async fn load_from_dir<P: AsRef<Path>>(dir_path: P) -> Result<AgentPersonaProfile> {
        let dir = dir_path.as_ref();
        let yaml_path = dir.join("profile.yaml");
        let json_path = dir.join("profile.json");

        if yaml_path.exists() {
            let content = fs::read_to_string(&yaml_path)
                .await
                .with_context(|| format!("Failed to read {}", yaml_path.display()))?;
            let profile: AgentPersonaProfile = serde_yaml_ng::from_str(&content)
                .with_context(|| format!("Failed to parse YAML in {}", yaml_path.display()))?;
            return Ok(profile);
        }

        if json_path.exists() {
            let content = fs::read_to_string(&json_path)
                .await
                .with_context(|| format!("Failed to read {}", json_path.display()))?;
            let profile: AgentPersonaProfile = serde_json::from_str(&content)
                .with_context(|| format!("Failed to parse JSON in {}", json_path.display()))?;
            return Ok(profile);
        }

        // Tự động phân tích từ các file Markdown (IDENTITY.md, SOUL.md, MISSION.md...)
        Self::load_from_markdown_files(dir).await
    }

    pub async fn load_from_markdown_files(dir: &Path) -> Result<AgentPersonaProfile> {
        let id_file = dir.join("IDENTITY.md");
        let soul_file = dir.join("SOUL.md");
        let mission_file = dir.join("MISSION.md");

        let id_content = if id_file.exists() {
            fs::read_to_string(&id_file).await.unwrap_or_default()
        } else {
            String::new()
        };

        let soul_content = if soul_file.exists() {
            fs::read_to_string(&soul_file).await.unwrap_or_default()
        } else {
            String::new()
        };

        let mission_content = if mission_file.exists() {
            fs::read_to_string(&mission_file).await.unwrap_or_default()
        } else {
            String::new()
        };

        let agent_name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("agent-default");

        let identity = AgentIdentity {
            id: AgentId::from_str(agent_name),
            name: format!("Agent {}", agent_name),
            description: if !id_content.is_empty() {
                id_content.lines().take(3).collect::<Vec<_>>().join(" ")
            } else {
                format!("Autonomous agent for {}", agent_name)
            },
            autonomy_level: AutonomyLevel::Full,
            created_at: chrono::Utc::now(),
        };

        let role = AgentRole {
            role_code: agent_name.to_uppercase(),
            title: format!("Specialist ({})", agent_name),
            primary_domain: "General Intelligence & Operations".to_string(),
            capabilities: vec![
                "autonomous reasoning".to_string(),
                "tool execution".to_string(),
            ],
        };

        let soul = AgentSoul {
            persona: if !soul_content.is_empty() {
                soul_content
            } else {
                "Professional, precise, rigorous, and proactive.".to_string()
            },
            tone_of_voice: "Objective and pragmatic".to_string(),
            core_values: vec![
                "Reliability".to_string(),
                "Safety & Guardrails".to_string(),
                "High Autonomy".to_string(),
            ],
            voice_profile: None,
        };

        let mission = AgentMission {
            high_level_objective: if !mission_content.is_empty() {
                mission_content
            } else {
                "Execute assigned goals with high fidelity and zero defect".to_string()
            },
            kpis: vec![
                "100% test pass rate".to_string(),
                "Zero red line violation".to_string(),
            ],
            constraints: vec![
                "Never execute destructive irreversible operations without approval".to_string(),
                "Strict verification after action".to_string(),
            ],
        };

        Ok(AgentPersonaProfile {
            identity,
            role,
            soul,
            mission,
        })
    }

    /// Xuất profile ra thư mục dưới dạng Markdown và JSON chuẩn
    pub async fn export_to_dir<P: AsRef<Path>>(
        profile: &AgentPersonaProfile,
        dir_path: P,
    ) -> Result<()> {
        let dir = dir_path.as_ref();
        tokio::fs::create_dir_all(dir).await?;

        // 1. profile.json
        let json_str = serde_json::to_string_pretty(profile)?;
        fs::write(dir.join("profile.json"), json_str).await?;

        // 2. IDENTITY.md
        let id_md = format!(
            "# {}\n\n- **Agent ID**: {}\n- **Title**: {}\n- **Domain**: {}\n- **Autonomy**: {:?}\n\n{}\n",
            profile.identity.name,
            profile.identity.id,
            profile.role.title,
            profile.role.primary_domain,
            profile.identity.autonomy_level,
            profile.identity.description
        );
        fs::write(dir.join("IDENTITY.md"), id_md).await?;

        // 3. SOUL.md
        let mut soul_md = format!(
            "# PERSONA & SOUL: {}\n\n{}\n\n### Tone of Voice\n{}\n\n### Core Values\n",
            profile.identity.name, profile.soul.persona, profile.soul.tone_of_voice
        );
        for cv in &profile.soul.core_values {
            soul_md.push_str(&format!("- {}\n", cv));
        }
        fs::write(dir.join("SOUL.md"), soul_md).await?;

        // 4. MISSION.md
        let mut mission_md = format!(
            "# MISSION & OBJECTIVES\n\n{}\n\n### Constraints & Safety Guardrails\n",
            profile.mission.high_level_objective
        );
        for c in &profile.mission.constraints {
            mission_md.push_str(&format!("- {}\n", c));
        }
        fs::write(dir.join("MISSION.md"), mission_md).await?;

        Ok(())
    }
}

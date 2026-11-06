use agent_domain::{
    AgentId, AgentIdentity, AgentMission, AgentPersonaProfile, AgentRole, AgentSoul,
    AutonomyLevel,
};
use anyhow::{Context, Result};
use std::path::Path;
use tokio::fs;

pub struct PersonaLoader;

impl PersonaLoader {
    /// Nạp cấu hình Agent từ một thư mục chứa profile:
    /// - `profile.yaml` hoặc `profile.json`
    ///
    /// HOẶC:
    /// - `IDENTITY.md`, `SOUL.md`, `MISSION.md`, `ROLE.md`
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

        let fallback_agent_name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("agent-default");

        // 1. Phân tích IDENTITY.md
        let mut parsed_name = None;
        let mut parsed_id = None;
        let mut parsed_title = None;
        let mut parsed_domain = None;
        let mut parsed_autonomy = AutonomyLevel::Full;
        let mut desc_lines = Vec::new();
        let mut in_metadata_list = false;

        for line in id_content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("# ") && parsed_name.is_none() {
                parsed_name = Some(trimmed.trim_start_matches("# ").trim().to_string());
                continue;
            }
            if trimmed.starts_with("- **Agent ID**:") {
                in_metadata_list = true;
                parsed_id = Some(trimmed.trim_start_matches("- **Agent ID**:").trim().to_string());
                continue;
            }
            if trimmed.starts_with("- **Title**:") {
                in_metadata_list = true;
                parsed_title = Some(trimmed.trim_start_matches("- **Title**:").trim().to_string());
                continue;
            }
            if trimmed.starts_with("- **Domain**:") {
                in_metadata_list = true;
                parsed_domain = Some(trimmed.trim_start_matches("- **Domain**:").trim().to_string());
                continue;
            }
            if trimmed.starts_with("- **Autonomy**:") {
                in_metadata_list = true;
                let auto_str = trimmed.trim_start_matches("- **Autonomy**:").trim();
                parsed_autonomy = match auto_str {
                    "Supervised" => AutonomyLevel::Supervised,
                    "ShadowMode" => AutonomyLevel::ShadowMode,
                    _ => AutonomyLevel::Full,
                };
                continue;
            }
            if in_metadata_list && (trimmed.is_empty() || trimmed.starts_with('-')) {
                continue;
            }
            if !trimmed.is_empty() {
                in_metadata_list = false;
                desc_lines.push(trimmed);
            }
        }

        let agent_name = parsed_name.unwrap_or_else(|| format!("Agent {}", fallback_agent_name));
        let agent_id = AgentId::from_str(&parsed_id.unwrap_or_else(|| fallback_agent_name.to_string()));
        let description = if !desc_lines.is_empty() {
            desc_lines.join(" ")
        } else {
            format!("Autonomous agent for {}", fallback_agent_name)
        };

        let identity = AgentIdentity {
            id: agent_id,
            name: agent_name,
            description,
            autonomy_level: parsed_autonomy,
            created_at: chrono::Utc::now(),
        };

        let role = AgentRole {
            role_code: fallback_agent_name.to_uppercase(),
            title: parsed_title.unwrap_or_else(|| format!("Specialist ({})", fallback_agent_name)),
            primary_domain: parsed_domain.unwrap_or_else(|| "General Intelligence & Operations".to_string()),
            capabilities: vec![
                "autonomous reasoning".to_string(),
                "tool execution".to_string(),
            ],
        };

        // 2. Phân tích SOUL.md
        let mut soul_persona_lines = Vec::new();
        let mut tone_lines = Vec::new();
        let mut core_values = Vec::new();
        let mut soul_section = 0; // 0: persona, 1: tone, 2: core values

        for line in soul_content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("# PERSONA & SOUL") {
                continue;
            }
            if trimmed.starts_with("### Tone of Voice") {
                soul_section = 1;
                continue;
            }
            if trimmed.starts_with("### Core Values") {
                soul_section = 2;
                continue;
            }

            match soul_section {
                0 => {
                    if !trimmed.is_empty() {
                        soul_persona_lines.push(trimmed);
                    }
                }
                1 => {
                    if !trimmed.is_empty() {
                        tone_lines.push(trimmed);
                    }
                }
                2 if trimmed.starts_with('-') => {
                    core_values.push(trimmed.trim_start_matches('-').trim().to_string());
                }
                _ => {}
            }
        }

        let persona = if !soul_persona_lines.is_empty() {
            soul_persona_lines.join("\n")
        } else {
            "Professional, precise, rigorous, and proactive.".to_string()
        };

        let tone_of_voice = if !tone_lines.is_empty() {
            tone_lines.join(" ")
        } else {
            "Objective and pragmatic".to_string()
        };

        let core_values = if !core_values.is_empty() {
            core_values
        } else {
            vec![
                "Reliability".to_string(),
                "Safety & Guardrails".to_string(),
                "High Autonomy".to_string(),
            ]
        };

        let soul = AgentSoul {
            persona,
            tone_of_voice,
            core_values,
            voice_profile: None,
        };

        // 3. Phân tích MISSION.md
        let mut mission_lines = Vec::new();
        let mut constraints = Vec::new();
        let mut mission_section = 0; // 0: objective, 1: constraints

        for line in mission_content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("# MISSION & OBJECTIVES") {
                continue;
            }
            if trimmed.starts_with("### Constraints") {
                mission_section = 1;
                continue;
            }

            match mission_section {
                0 => {
                    if !trimmed.is_empty() {
                        mission_lines.push(trimmed);
                    }
                }
                1 if trimmed.starts_with('-') => {
                    constraints.push(trimmed.trim_start_matches('-').trim().to_string());
                }
                _ => {}
            }
        }

        let high_level_objective = if !mission_lines.is_empty() {
            mission_lines.join("\n")
        } else {
            "Execute assigned goals with high fidelity and zero defect".to_string()
        };

        let constraints = if !constraints.is_empty() {
            constraints
        } else {
            vec![
                "Never execute destructive irreversible operations without approval".to_string(),
                "Strict verification after action".to_string(),
            ]
        };

        let mission = AgentMission {
            high_level_objective,
            kpis: vec![
                "100% test pass rate".to_string(),
                "Zero red line violation".to_string(),
            ],
            constraints,
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

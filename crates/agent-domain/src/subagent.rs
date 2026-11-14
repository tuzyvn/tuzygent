use crate::rig::{RigMetadata, RigOperatorConfig, RigSeatConfig, RigSpec};
use serde::{Deserialize, Deserializer, Serialize};
use std::path::Path;

fn deserialize_string_or_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrVec {
        Str(String),
        Vec(Vec<String>),
    }
    match StringOrVec::deserialize(deserializer)? {
        StringOrVec::Str(s) => Ok(s
            .split(',')
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToString::to_string)
            .collect()),
        StringOrVec::Vec(v) => Ok(v),
    }
}

/// YAML frontmatter for modern file-system Markdown subagents (`agents/*.md`, `.claude/agents/*.md`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SubagentFrontmatter {
    pub id: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub role: Option<String>,
    #[serde(default, deserialize_with = "deserialize_string_or_vec")]
    pub capabilities: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_string_or_vec")]
    pub tools: Vec<String>,
    pub engine: Option<String>,
    pub model: Option<String>,
    pub harness: Option<String>,
    pub autonomy: Option<String>,
}

/// Parsed Markdown subagent specification pairing routing metadata with an isolated system prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentSpec {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub engine: Option<String>,
    pub model: Option<String>,
    pub harness: String,
    pub autonomy: String,
    pub system_prompt: String,
    pub source_path: Option<String>,
}

fn split_frontmatter(markdown: &str) -> Result<(SubagentFrontmatter, String), String> {
    let normalized = markdown.replace("\r\n", "\n");
    let trimmed = normalized.trim_start_matches('\u{feff}');
    if let Some(after_open) = trimmed
        .strip_prefix("---\n")
        .or_else(|| trimmed.strip_prefix("---"))
    {
        if let Some(close_idx) = after_open.find("\n---") {
            let yaml_part = &after_open[..close_idx];
            let rest = &after_open[close_idx + 4..];
            let body = rest.strip_prefix('\n').unwrap_or(rest).trim().to_string();
            let fm = serde_yaml_ng::from_str::<SubagentFrontmatter>(yaml_part)
                .map_err(|e| format!("Invalid subagent YAML frontmatter: {e}"))?;
            return Ok((fm, body));
        }
    }
    Ok((SubagentFrontmatter::default(), trimmed.trim().to_string()))
}

fn slugify_id(raw: &str) -> String {
    let mut slug = String::with_capacity(raw.len());
    let mut prev_dash = false;
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !slug.is_empty() {
            slug.push('-');
            prev_dash = true;
        }
    }
    slug.trim_matches('-').to_string()
}

impl SubagentSpec {
    /// Parse a Markdown subagent definition (`--- frontmatter ---\n\nsystem prompt`).
    pub fn from_markdown_str(
        fallback_id: &str,
        markdown: &str,
        source_path: Option<&str>,
    ) -> Result<Self, String> {
        let (fm, system_prompt) = split_frontmatter(markdown)?;
        let raw_id = fm
            .id
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| fm.name.as_deref().filter(|s| !s.trim().is_empty()))
            .unwrap_or(fallback_id);
        let id = slugify_id(raw_id);
        let id = if id.is_empty() {
            "subagent".to_string()
        } else {
            id
        };
        let name = fm
            .name
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| id.clone());
        let description = fm
            .description
            .filter(|s| !s.trim().is_empty())
            .or_else(|| fm.role.filter(|s| !s.trim().is_empty()))
            .unwrap_or_else(|| "Specialist subagent".to_string());
        let mut capabilities = fm.capabilities;
        for tool in fm.tools {
            if !capabilities.iter().any(|c| c.eq_ignore_ascii_case(&tool)) {
                capabilities.push(tool);
            }
        }
        Ok(Self {
            id,
            name,
            description,
            capabilities,
            engine: fm.engine.filter(|s| !s.trim().is_empty()),
            model: fm.model.filter(|s| !s.trim().is_empty()),
            harness: fm
                .harness
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "process".to_string()),
            autonomy: fm
                .autonomy
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "full".to_string()),
            system_prompt,
            source_path: source_path.map(ToString::to_string),
        })
    }

    /// Render this subagent spec back to YAML-frontmatter Markdown.
    pub fn to_markdown(&self) -> Result<String, String> {
        let fm = SubagentFrontmatter {
            id: Some(self.id.clone()),
            name: Some(self.name.clone()),
            description: Some(self.description.clone()),
            role: None,
            capabilities: self.capabilities.clone(),
            tools: Vec::new(),
            engine: self.engine.clone(),
            model: self.model.clone(),
            harness: Some(self.harness.clone()),
            autonomy: Some(self.autonomy.clone()),
        };
        let yaml = serde_yaml_ng::to_string(&fm)
            .map_err(|e| format!("Failed to serialize subagent frontmatter: {e}"))?;
        Ok(format!("---\n{yaml}---\n\n{}\n", self.system_prompt.trim()))
    }

    /// Adapt this subagent roster entry to a runtime `RigSeatConfig` without static channels.
    pub fn to_seat_config(&self) -> RigSeatConfig {
        RigSeatConfig {
            seat_id: self.id.clone(),
            name: Some(self.name.clone()),
            role: self.description.clone(),
            capabilities: self.capabilities.clone(),
            persona_path: self.source_path.clone(),
            engine: self.engine.clone(),
            model: self.model.clone(),
            harness: self.harness.clone(),
            voice_preset: None,
            autonomy: self.autonomy.clone(),
        }
    }
}

/// Load all `*.md` subagent specs in `dir`, sorted deterministically by file stem.
pub fn load_subagents_from_dir(dir: &Path) -> Result<Vec<SubagentSpec>, String> {
    let read_dir = std::fs::read_dir(dir)
        .map_err(|e| format!("Failed to read subagent dir '{}': {e}", dir.display()))?;
    let mut md_paths: Vec<std::path::PathBuf> = read_dir
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("md")))
        .collect();
    md_paths.sort();

    let mut specs = Vec::with_capacity(md_paths.len());
    for path in md_paths {
        let fallback_id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("subagent");
        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read subagent '{}': {e}", path.display()))?;
        let source = path.to_string_lossy().to_string();
        specs.push(SubagentSpec::from_markdown_str(
            fallback_id,
            &content,
            Some(&source),
        )?);
    }
    Ok(specs)
}

/// Built-in 3-specialist roster used when a workspace has no custom `agents/*.md` files.
pub fn default_subagent_roster() -> Vec<SubagentSpec> {
    vec![
        SubagentSpec {
            id: "architect".to_string(),
            name: "Architect".to_string(),
            description: "System design, boundaries, and read-only technical consultation".to_string(),
            capabilities: vec!["architecture".to_string(), "design".to_string(), "review".to_string()],
            engine: None,
            model: None,
            harness: "process".to_string(),
            autonomy: "full".to_string(),
            system_prompt: "Analyze requirements, module boundaries, and risks concisely. Never invent file paths or APIs you did not inspect.".to_string(),
            source_path: None,
        },
        SubagentSpec {
            id: "developer".to_string(),
            name: "Developer".to_string(),
            description: "Implement code changes, bug fixes, and targeted tests".to_string(),
            capabilities: vec!["code".to_string(), "rust".to_string(), "fix".to_string(), "implement".to_string()],
            engine: None,
            model: None,
            harness: "process".to_string(),
            autonomy: "full".to_string(),
            system_prompt: "Implement clean, minimal patches that satisfy the requested done_when criteria and verify them with tests.".to_string(),
            source_path: None,
        },
        SubagentSpec {
            id: "reviewer".to_string(),
            name: "Reviewer".to_string(),
            description: "Verify code quality, test coverage, and security invariants".to_string(),
            capabilities: vec!["review".to_string(), "verify".to_string(), "security".to_string(), "qa".to_string()],
            engine: None,
            model: None,
            harness: "process".to_string(),
            autonomy: "full".to_string(),
            system_prompt: "Inspect diffs and test results critically. Flag missing edge cases, regressions, or unmet done_when criteria.".to_string(),
            source_path: None,
        },
    ]
}

/// Convert a subagent roster into a modern `RigSpec` (no static `channels` wiring).
pub fn roster_to_rig_spec(rig_id: &str, rig_name: &str, specs: &[SubagentSpec]) -> RigSpec {
    RigSpec {
        rig: RigMetadata {
            id: rig_id.to_string(),
            name: rig_name.to_string(),
            description: Some("Dynamic Markdown subagent roster".to_string()),
            version: None,
        },
        operator: RigOperatorConfig {
            agent_id: "lead-operator".to_string(),
            name: Some("Lead Operator".to_string()),
            role: Some("Dynamic Coordinator".to_string()),
            persona_path: None,
            engine: None,
            model: None,
        },
        seats: specs.iter().map(SubagentSpec::to_seat_config).collect(),
        channels: Vec::new(),
    }
}

/// Auto-discover workspace Markdown subagents (`agents/`, `.tuzygent/agents/`, `.claude/agents/`), falling back to the built-in roster.
pub fn discover_workspace_roster(workspace_dir: &Path) -> Result<(Vec<SubagentSpec>, RigSpec), String> {
    let candidates = [
        workspace_dir.join("agents"),
        workspace_dir.join(".tuzygent").join("agents"),
        workspace_dir.join(".claude").join("agents"),
    ];
    for dir in &candidates {
        if dir.is_dir() {
            let specs = load_subagents_from_dir(dir)?;
            if !specs.is_empty() {
                let rig = roster_to_rig_spec("workspace-roster", "Workspace Subagent Roster", &specs);
                return Ok((specs, rig));
            }
        }
    }
    let specs = default_subagent_roster();
    let rig = roster_to_rig_spec("builtin-roster", "Built-in Subagent Roster", &specs);
    Ok((specs, rig))
}

use crate::skills::SkillMetadata;
use agent_domain::AgentPersonaProfile;

/// ContextBuilder unifies and structures execution context across
/// Persona, Goals, Skills, Lessons, Recent Scratchpad Logs, Subtask History, and Artifacts.
/// It enforces explicit character limits per component to prevent prompt bloat.
#[derive(Debug, Clone)]
pub struct ContextBuilder {
    goal: String,
    persona: Option<AgentPersonaProfile>,
    skills: Vec<SkillMetadata>,
    lessons: Vec<String>,
    recent_logs: Vec<String>,
    subtask_history: Vec<(String, String)>, // (subtask_id, excerpt)
    artifacts: Vec<String>,
    max_total_chars: usize,
}

impl ContextBuilder {
    pub fn new(goal: impl Into<String>) -> Self {
        Self {
            goal: goal.into(),
            persona: None,
            skills: Vec::new(),
            lessons: Vec::new(),
            recent_logs: Vec::new(),
            subtask_history: Vec::new(),
            artifacts: Vec::new(),
            max_total_chars: 16000,
        }
    }

    pub fn with_persona(mut self, persona: Option<AgentPersonaProfile>) -> Self {
        self.persona = persona;
        self
    }

    pub fn with_skills(mut self, skills: Vec<SkillMetadata>) -> Self {
        self.skills = skills;
        self
    }

    pub fn with_lessons(mut self, lessons: Vec<String>) -> Self {
        self.lessons = lessons;
        self
    }

    pub fn with_recent_logs(mut self, logs: Vec<String>) -> Self {
        self.recent_logs = logs;
        self
    }

    pub fn with_subtask_history(mut self, history: Vec<(String, String)>) -> Self {
        self.subtask_history = history;
        self
    }

    pub fn add_subtask_step(mut self, id: String, excerpt: String) -> Self {
        self.subtask_history.push((id, excerpt));
        self
    }

    pub fn with_artifacts(mut self, artifacts: Vec<String>) -> Self {
        self.artifacts = artifacts;
        self
    }

    pub fn with_max_total_chars(mut self, max_chars: usize) -> Self {
        self.max_total_chars = max_chars;
        self
    }

    /// Assembles the unified (system_context, working_context)
    pub fn build(self) -> (String, String) {
        let mut sys = String::new();

        // 1. Persona section (up to 2500 chars)
        if let Some(p) = &self.persona {
            sys.push_str("### AGENT IDENTITY & VOICE:\n");
            sys.push_str(&p.build_system_prompt());
            sys.push_str("\n\n");
        }

        // 2. Goal Section
        sys.push_str("### PRIMARY GOAL:\n");
        sys.push_str(&self.goal);
        sys.push_str("\n\n");

        // 3. Relevant Skills Section (up to 2500 chars)
        if !self.skills.is_empty() {
            sys.push_str("### RELEVANT OPERATIONAL SKILLS:\n");
            for skill in &self.skills {
                let desc: String = skill.description.chars().take(200).collect();
                sys.push_str(&format!("- [{}] {}: {}\n", skill.id, skill.name, desc));
            }
            sys.push('\n');
        }

        // 4. Lessons Learned (up to 2500 chars)
        if !self.lessons.is_empty() {
            sys.push_str("### DURABLE LESSONS & GUARDRAILS:\n");
            for l in &self.lessons {
                sys.push_str(l);
                sys.push('\n');
            }
            sys.push('\n');
        }

        // Assemble Working Context
        let mut work = String::new();

        // 5. Recent Scratchpad Logs (up to 1500 chars)
        if !self.recent_logs.is_empty() {
            work.push_str("### RECENT SCRATCHPAD ACTIVITY:\n");
            for log in &self.recent_logs {
                work.push_str(&format!("* {}\n", log));
            }
            work.push('\n');
        }

        // 6. Subtask Execution History (up to 4000 chars)
        if !self.subtask_history.is_empty() {
            work.push_str("### PREVIOUS SUBTASK OUTPUTS:\n");
            for (step_id, output) in &self.subtask_history {
                let trimmed_out: String = output.chars().take(800).collect();
                work.push_str(&format!("#### Step {}:\n{}\n\n", step_id, trimmed_out));
            }
        }

        // 7. Tracked Artifacts
        if !self.artifacts.is_empty() {
            work.push_str("### CREATED ARTIFACTS:\n");
            for art in &self.artifacts {
                work.push_str(&format!("- {}\n", art));
            }
            work.push('\n');
        }

        // Ensure total length complies with limit
        let total = sys.len() + work.len();
        if total > self.max_total_chars {
            let available_work = self.max_total_chars.saturating_sub(sys.len());
            work = work.chars().rev().take(available_work).collect::<String>().chars().rev().collect();
        }

        (sys, work)
    }
}

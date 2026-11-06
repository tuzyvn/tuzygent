use crate::sandbox::{SandboxConfig, SandboxDriverKind};
use agent_domain::{GuardrailPolicy, SubTask};
use agent_engine::{ComputeEngine, EngineError, EngineInput, EngineOutput};
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use tracing::{info, info_span, warn, Instrument};

/// Dynamic goal planning and subtask decomposition.
#[derive(Clone)]
pub struct GoalDecomposer {
    engine: Arc<dyn ComputeEngine>,
    guardrail: GuardrailPolicy,
    workspace_dir: String,
    sandbox_config: Option<SandboxConfig>,
    timeout_seconds: u64,
    backoff_policy: Option<agent_engine::BackoffPolicy>,
}

impl GoalDecomposer {
    pub fn new(
        engine: Arc<dyn ComputeEngine>,
        guardrail: GuardrailPolicy,
        workspace_dir: impl Into<String>,
        sandbox_config: Option<SandboxConfig>,
        timeout_seconds: u64,
    ) -> Self {
        Self {
            engine,
            guardrail,
            workspace_dir: workspace_dir.into(),
            sandbox_config,
            timeout_seconds,
            backoff_policy: None,
        }
    }

    pub fn with_backoff_policy(mut self, policy: agent_engine::BackoffPolicy) -> Self {
        self.backoff_policy = Some(policy);
        self
    }

    pub fn backoff_policy(&self) -> Option<&agent_engine::BackoffPolicy> {
        self.backoff_policy.as_ref()
    }

    pub async fn execute_engine(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
        let mut service = crate::execution::ExecutionService::new(
            &self.workspace_dir,
            self.sandbox_config.clone(),
            Some(self.guardrail.clone()),
        );
        if let Some(ref policy) = self.backoff_policy {
            service = service.with_backoff_policy(policy.clone()).with_max_retries(3);
        }
        service.execute(self.engine.clone(), input).await
    }

    /// Tự động phân rã Goal thành cây SubTasks động qua ComputeEngine (không gán cứng)
    pub async fn decompose_goal(
        &self,
        task_title: &str,
        goal_description: &str,
        workdir: &Path,
        system_context: &str,
    ) -> Vec<SubTask> {
        let span = info_span!(
            "ooda_phase",
            task_id = "",
            subtask_id = "",
            execution_phase = super::OODA_PHASE_DECIDE,
        );
        async move {
            info!("🧠 [OODA Planner] Phân tích mục tiêu và phân rã Subtasks động qua LLM...");

        let planning_prompt = format!(
            "You are an Autonomous AI System Architect and Planner.\n\
            Goal Title: {title}\n\
            Goal Description: {desc}\n\
            Workspace Directory: {workdir}\n\n\
            Analyze the requirement and decompose it into 2 to 5 concrete, actionable, sequential subtasks.\n\
            For each subtask, specify:\n\
            - \"id\": Unique ID (e.g. \"ST-1\", \"ST-2\")\n\
            - \"title\": Brief action title\n\
            - \"command_or_prompt\": Exact command or concrete instruction to execute\n\
            - \"expected_verification\": Specific condition or metric to verify completion\n\n\
            Output ONLY a valid JSON array of objects conforming to the schema above. Do not include markdown preamble or conversational text.",
            title = task_title,
            desc = goal_description,
            workdir = if self.sandbox_config.as_ref().is_some_and(|config| config.driver == SandboxDriverKind::Docker) {
                "/workspace".to_string()
            } else { workdir.display().to_string() },
        );

        let input = EngineInput {
            prompt_or_command: planning_prompt,
            system_context: system_context.to_string(),
            working_dir: workdir.to_string_lossy().to_string(),
            timeout_seconds: self.timeout_seconds.min(60),
            on_stream_chunk: None,
            command_sandbox: None,
            on_event: None,
            temperature: None,
            max_tokens: None,
            max_output_bytes: Some(self.guardrail.max_output_bytes),
        };

        let mut subtasks = match self.execute_engine(&input).await {
            Ok(out) => {
                let raw = out.stdout_content.trim();
                if let Some(parsed) = Self::parse_subtasks_json(raw) {
                    if !parsed.is_empty() {
                        info!(
                            "📋 [OODA Planner] Đã tự phân rã thành công {} Subtasks động.",
                            parsed.len()
                        );
                        parsed
                    } else {
                        vec![SubTask::new(
                            "ST-1",
                            task_title,
                            goal_description,
                            "Execution completed with zero errors",
                        )]
                    }
                } else {
                    vec![SubTask::new(
                        "ST-1",
                        task_title,
                        goal_description,
                        "Execution completed with zero errors",
                    )]
                }
            }
            Err(e) => {
                warn!(
                    "⚠️ [OODA Planner] Engine execution failed during decomposition: {}. Falling back to default subtask.",
                    e
                );
                vec![SubTask::new(
                    "ST-1",
                    task_title,
                    goal_description,
                    "Execution completed with zero errors",
                )]
            }
        };

        // Enforce guardrail policy: max_subtasks_per_goal
        if subtasks.len() > self.guardrail.max_subtasks_per_goal {
            warn!(
                "⚠️ [OODA Planner] Subtasks count ({}) exceeded policy limit ({}). Truncating.",
                subtasks.len(),
                self.guardrail.max_subtasks_per_goal
            );
            subtasks.truncate(self.guardrail.max_subtasks_per_goal);
        }

        // Deduplicate subtask IDs
        let mut seen = HashSet::new();
        for (i, st) in subtasks.iter_mut().enumerate() {
            if !seen.insert(st.id.clone()) || st.id.is_empty() {
                st.id = format!("ST-{}", i + 1);
                seen.insert(st.id.clone());
            }
        }

            subtasks
        }
        .instrument(span)
        .await
    }

    /// Trích xuất và parse mảng JSON SubTask từ văn bản LLM
    pub fn parse_subtasks_json(raw: &str) -> Option<Vec<SubTask>> {
        let json_text = if let Some(start) = raw.find("```json") {
            let after = raw.get(start + 7..).unwrap_or("");
            if let Some(end) = after.find("```") {
                after.get(..end).unwrap_or(after).trim()
            } else {
                after.trim()
            }
        } else if let Some(start) = raw.find("```") {
            let after = raw.get(start + 3..).unwrap_or("");
            if let Some(end) = after.find("```") {
                after.get(..end).unwrap_or(after).trim()
            } else {
                after.trim()
            }
        } else if let (Some(start), Some(end)) = (raw.find('['), raw.rfind(']')) {
            if end >= start {
                raw.get(start..=end).unwrap_or(raw)
            } else {
                raw
            }
        } else {
            raw
        };

        let val: Value = serde_json::from_str(json_text).ok()?;
        let arr = match val.as_array() {
            Some(a) => a,
            None => val
                .get("subtasks")
                .or_else(|| val.get("tasks"))
                .or_else(|| val.get("plan"))
                .and_then(|v| v.as_array())?,
        };
        let mut subtasks = Vec::new();

        for (idx, item) in arr.iter().enumerate() {
            let id = item
                .get("id")
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("ST-{}", idx + 1));
            let title = item
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("Execute subtask")
                .to_string();
            let command_or_prompt = item
                .get("command_or_prompt")
                .or_else(|| item.get("command"))
                .or_else(|| item.get("prompt"))
                .and_then(|v| v.as_str())
                .unwrap_or(&title)
                .to_string();
            let expected_verification = item
                .get("expected_verification")
                .or_else(|| item.get("verification"))
                .and_then(|v| v.as_str())
                .unwrap_or("Status code 0")
                .to_string();

            subtasks.push(SubTask::new(
                &id,
                &title,
                &command_or_prompt,
                &expected_verification,
            ));
        }

        Some(subtasks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_subtasks_json_codeblock() {
        let raw = r#"Here is the plan:
```json
[
  {
    "id": "ST-1",
    "title": "Check code formatting",
    "command_or_prompt": "cargo fmt --check",
    "expected_verification": "Zero formatting errors"
  },
  {
    "id": "ST-2",
    "title": "Run tests",
    "command_or_prompt": "cargo test",
    "expected_verification": "Status code 0"
  }
]
```
Done!"#;

        let parsed = GoalDecomposer::parse_subtasks_json(raw).expect("should parse subtasks");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].id, "ST-1");
        assert_eq!(parsed[0].title, "Check code formatting");
        assert_eq!(parsed[0].command_or_prompt, "cargo fmt --check");
        assert_eq!(parsed[0].expected_verification, "Zero formatting errors");
        assert_eq!(parsed[1].id, "ST-2");
    }

    #[test]
    fn test_parse_subtasks_json_fallback_keys() {
        let raw = r#"[
  {
    "command": "npm test",
    "verification": "all pass"
  }
]"#;

        let parsed = GoalDecomposer::parse_subtasks_json(raw).expect("should parse subtasks");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, "ST-1");
        assert_eq!(parsed[0].command_or_prompt, "npm test");
        assert_eq!(parsed[0].expected_verification, "all pass");
    }

    #[test]
    fn test_parse_subtasks_json_object_wrapper() {
        let raw = r#"{
  "subtasks": [
    {
      "id": "ST-WRAP-1",
      "title": "Wrapped task",
      "command": "cargo check",
      "verification": "exit 0"
    }
  ]
}"#;

        let parsed = GoalDecomposer::parse_subtasks_json(raw).expect("should parse wrapped subtasks");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, "ST-WRAP-1");
        assert_eq!(parsed[0].title, "Wrapped task");

        let plan_raw = r#"{
  "plan": [
    {
      "title": "Planned step",
      "command": "cargo test"
    }
  ]
}"#;
        let plan_parsed = GoalDecomposer::parse_subtasks_json(plan_raw).expect("should parse plan subtasks");
        assert_eq!(plan_parsed.len(), 1);
        assert_eq!(plan_parsed[0].id, "ST-1");
    }

    #[test]
    fn test_parse_subtasks_json_invalid() {
        let raw = "Not json at all";
        assert!(GoalDecomposer::parse_subtasks_json(raw).is_none());
    }
}

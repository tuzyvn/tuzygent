use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// State of an individual step in the execution plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum StepExecutionState {
    Pending,
    Running {
        started_at: DateTime<Utc>,
    },
    Completed {
        completed_at: DateTime<Utc>,
        output_summary: Option<String>,
    },
    Failed {
        failed_at: DateTime<Utc>,
        error: String,
        retries: u32,
    },
    Skipped {
        reason: String,
    },
}

impl std::fmt::Display for StepExecutionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StepExecutionState::Pending => write!(f, "PENDING"),
            StepExecutionState::Running { .. } => write!(f, "RUNNING"),
            StepExecutionState::Completed { .. } => write!(f, "COMPLETED"),
            StepExecutionState::Failed { error, retries, .. } => {
                write!(f, "FAILED (retries: {}, error: {})", retries, error)
            }
            StepExecutionState::Skipped { reason } => write!(f, "SKIPPED ({})", reason),
        }
    }
}

/// A discrete step in an auditable multi-agent execution plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionPlanStep {
    pub id: String,
    pub title: String,
    pub description: String,
    pub status: StepExecutionState,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assigned_agent: Option<String>,
}

impl ExecutionPlanStep {
    pub fn new(id: &str, title: &str, description: &str) -> Self {
        Self {
            id: id.to_string(),
            title: title.to_string(),
            description: description.to_string(),
            status: StepExecutionState::Pending,
            dependencies: Vec::new(),
            assigned_agent: None,
        }
    }

    pub fn with_dependency(mut self, dep_id: &str) -> Self {
        self.dependencies.push(dep_id.to_string());
        self
    }

    pub fn with_agent(mut self, agent_id: &str) -> Self {
        self.assigned_agent = Some(agent_id.to_string());
        self
    }

    pub fn is_completed(&self) -> bool {
        matches!(self.status, StepExecutionState::Completed { .. })
    }
}

/// Audit trail record tracking step state changes over time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanAuditRecord {
    pub timestamp: DateTime<Utc>,
    pub step_id: String,
    pub from_state: String,
    pub to_state: String,
    pub reason: String,
}

/// First-class auditable execution plan coordinating multi-agent steps.
/// Inspired by Orkas's dynamic ExecutionPlan state machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionPlan {
    pub plan_id: String,
    pub title: String,
    pub version: u32,
    pub steps: Vec<ExecutionPlanStep>,
    pub audit_history: Vec<PlanAuditRecord>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ExecutionPlan {
    pub fn new(plan_id: &str, title: &str) -> Self {
        let now = Utc::now();
        Self {
            plan_id: plan_id.to_string(),
            title: title.to_string(),
            version: 1,
            steps: Vec::new(),
            audit_history: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    pub fn add_step(&mut self, step: ExecutionPlanStep) {
        self.steps.push(step);
        self.updated_at = Utc::now();
    }

    /// Transitions a step to a new state and appends a deterministic audit trail entry.
    pub fn transition_step(
        &mut self,
        step_id: &str,
        new_state: StepExecutionState,
        reason: &str,
    ) -> bool {
        let now = Utc::now();
        if let Some(step) = self.steps.iter_mut().find(|s| s.id == step_id) {
            let from_str = step.status.to_string();
            let to_str = new_state.to_string();

            self.audit_history.push(PlanAuditRecord {
                timestamp: now,
                step_id: step_id.to_string(),
                from_state: from_str,
                to_state: to_str,
                reason: reason.to_string(),
            });

            step.status = new_state;
            self.version += 1;
            self.updated_at = now;
            true
        } else {
            false
        }
    }

    /// Identifies all steps ready to be executed (dependencies all Completed).
    pub fn executable_steps(&self) -> Vec<&ExecutionPlanStep> {
        let completed_ids: std::collections::HashSet<&str> = self
            .steps
            .iter()
            .filter(|s| s.is_completed())
            .map(|s| s.id.as_str())
            .collect();

        self.steps
            .iter()
            .filter(|s| matches!(s.status, StepExecutionState::Pending))
            .filter(|s| {
                s.dependencies
                    .iter()
                    .all(|dep| completed_ids.contains(dep.as_str()))
            })
            .collect()
    }

    /// Returns true if all steps in the plan have reached a terminal state.
    pub fn is_finished(&self) -> bool {
        !self.steps.is_empty()
            && self.steps.iter().all(|s| {
                matches!(
                    s.status,
                    StepExecutionState::Completed { .. }
                        | StepExecutionState::Failed { .. }
                        | StepExecutionState::Skipped { .. }
                )
            })
    }

    /// Returns true if every step completed successfully.
    pub fn is_all_successful(&self) -> bool {
        !self.steps.is_empty() && self.steps.iter().all(|s| s.is_completed())
    }

    /// Renders the execution plan as a Markdown table with status icons.
    pub fn to_markdown_table(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "### Execution Plan: {} (v{})\n\n",
            self.title, self.version
        ));
        out.push_str("| ID | Title | Status | Agent | Dependencies |\n");
        out.push_str("|---|---|---|---|---|\n");

        for step in &self.steps {
            let status_badge = match &step.status {
                StepExecutionState::Pending => "⏳ Pending",
                StepExecutionState::Running { .. } => "🔄 Running",
                StepExecutionState::Completed { .. } => "✅ Completed",
                StepExecutionState::Failed { .. } => "❌ Failed",
                StepExecutionState::Skipped { .. } => "⏭️ Skipped",
            };
            let agent = step.assigned_agent.as_deref().unwrap_or("-");
            let deps = if step.dependencies.is_empty() {
                "-".to_string()
            } else {
                step.dependencies.join(", ")
            };
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                step.id, step.title, status_badge, agent, deps
            ));
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execution_plan_lifecycle_and_dag_resolution() {
        let mut plan = ExecutionPlan::new("PLAN-001", "Database Migration & Seed");

        plan.add_step(ExecutionPlanStep::new("S1", "Create Tables", "Run migration scripts"));
        plan.add_step(
            ExecutionPlanStep::new("S2", "Seed Initial Users", "Insert seed data")
                .with_dependency("S1"),
        );
        plan.add_step(
            ExecutionPlanStep::new("S3", "Generate Cache Warmup", "Warm Redis cache")
                .with_dependency("S2")
                .with_agent("cache-bot"),
        );

        // Initially only S1 has no dependencies
        let ready1 = plan.executable_steps();
        assert_eq!(ready1.len(), 1);
        assert_eq!(ready1[0].id, "S1");

        // Start S1
        plan.transition_step(
            "S1",
            StepExecutionState::Running {
                started_at: Utc::now(),
            },
            "Worker spawned",
        );
        assert_eq!(plan.executable_steps().len(), 0);

        // Complete S1
        plan.transition_step(
            "S1",
            StepExecutionState::Completed {
                completed_at: Utc::now(),
                output_summary: Some("3 tables created".into()),
            },
            "Migrations applied",
        );

        // Now S2 should be executable
        let ready2 = plan.executable_steps();
        assert_eq!(ready2.len(), 1);
        assert_eq!(ready2[0].id, "S2");

        // Complete S2 and S3
        plan.transition_step(
            "S2",
            StepExecutionState::Completed {
                completed_at: Utc::now(),
                output_summary: None,
            },
            "Users seeded",
        );
        plan.transition_step(
            "S3",
            StepExecutionState::Completed {
                completed_at: Utc::now(),
                output_summary: None,
            },
            "Cache warmed",
        );

        assert!(plan.is_all_successful());
        assert!(plan.is_finished());
        assert_eq!(plan.audit_history.len(), 4);
        assert_eq!(plan.version, 5);

        let table = plan.to_markdown_table();
        assert!(table.contains("Database Migration & Seed"));
        assert!(table.contains("✅ Completed"));
        assert!(table.contains("cache-bot"));
    }
}

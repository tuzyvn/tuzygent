use super::super::TaskService;
use agent_domain::TaskStatus;
use anyhow::Result;

impl TaskService {
    /// Call once on startup, with exclusive ownership of the workspace's task database.
    /// Running work is never replayed automatically because its side effects are unknown.
    pub async fn recover_after_restart(&self) -> Result<()> {
        let config = agent_engine::EnginesConfig::load_hierarchical(Some(&self.workspace_dir));
        let mut offset = 0;
        loop {
            let tasks = self.store.list_tasks(100, offset).await?;
            if tasks.is_empty() {
                break;
            }
            offset += tasks.len();
            for mut task in tasks {
                match task.status {
                    TaskStatus::Queued => {
                        let engine = task.engine_name.as_deref().and_then(|name| {
                            agent_engine::configured_engine(&config, name, None).ok()
                        });
                        if let Some(engine) = engine {
                            self.dispatch(task.id.clone(), engine);
                        } else {
                            task.status = TaskStatus::WaitingApproval {
                                reason: "Recovery requires a valid recorded engine".into(),
                            };
                            task.updated_at = chrono::Utc::now();
                            let _ = self.store.save_task(&task).await;
                        }
                    }
                    TaskStatus::Executing
                    | TaskStatus::PreparingSandbox
                    | TaskStatus::SelfChecking
                    | TaskStatus::Supplementing { .. }
                    | TaskStatus::Dispatched { .. }
                    | TaskStatus::Canceled { .. } => {
                        match self.store.get_task_checkpoint(&task.id).await {
                            Ok(Some(mut cp)) => {
                                cp.sanitize_for_resume();
                                task.subtasks = cp.subtasks;
                            }
                            Ok(None) => {}
                            Err(e) => {
                                tracing::warn!("Interrupted task {} has corrupted checkpoint during recovery: {}", task.id, e);
                            }
                        }
                        task.status = TaskStatus::WaitingApproval {
                            reason: "Interrupted execution requires reconciliation before retry"
                                .into(),
                        };
                        task.updated_at = chrono::Utc::now();
                        let _ = self.store.save_task(&task).await;
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
}

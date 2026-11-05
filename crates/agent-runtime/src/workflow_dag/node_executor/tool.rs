use chrono::Utc;
use tokio::process::Command;

use super::types::NodeExecutionOutcome;
use crate::harness::{HarnessMemoryStore, HarnessSelfHealer};
use crate::workflow_dag::types::{WorkflowNode, WorkflowTimelineEvent};

pub async fn execute_tool_node(
    workspace_dir: &str,
    harness_memory: &HarnessMemoryStore,
    node: &WorkflowNode,
    current_id: &str,
    node_label: &str,
    timeline: &mut Vec<WorkflowTimelineEvent>,
) -> NodeExecutionOutcome {
    let mut node_output = serde_json::json!({});
    let mut node_error: Option<String> = None;
    let mut status = "completed".to_string();
    let mut healed: Option<bool> = None;
    let mut healing_attempts: Option<u32> = None;
    let mut lesson_learned: Option<String> = None;

    if let Some(cmd_str) = &node.data.command {
        let self_healing_enabled = node.data.enable_self_healing.unwrap_or(true);
        let mut current_cmd = cmd_str.clone();
        let mut tool_attempt = 0;
        let max_tool_attempts = 1;

        loop {
            let mut cmd = Command::new("bash");
            cmd.arg("-c").arg(&current_cmd);
            cmd.current_dir(workspace_dir);
            if let Err(error) =
                agent_domain::GuardrailPolicy::default_enterprise().check_command_safety(&current_cmd)
            {
                status = "waiting_approval".into();
                node_error = Some(error);
                break;
            }

            match agent_engine::process::run_process(
                cmd,
                std::time::Duration::from_secs(60),
                None,
            )
            .await
            {
                Ok(output) => {
                    let stdout_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    let stderr_str = String::from_utf8_lossy(&output.stderr).trim().to_string();
                    let exit_code = output.status.code().unwrap_or(-1);

                    let parsed_json: serde_json::Value =
                        serde_json::from_str(&stdout_str).unwrap_or_else(|_| serde_json::json!({ "raw": stdout_str }));

                    node_output = serde_json::json!({
                        "status": exit_code,
                        "stdout": stdout_str,
                        "stderr": stderr_str,
                        "data": parsed_json
                    });

                    if exit_code == 0 {
                        status = "completed".to_string();
                        node_error = None;
                        if tool_attempt > 0 {
                            healed = Some(true);
                            healing_attempts = Some(tool_attempt);
                            let remediation = format!("Điều chỉnh lệnh thực thi sang: {}", current_cmd);
                            let _ = harness_memory
                                .record_lesson("toolNode", current_id, "Command execution failure", &remediation)
                                .await;
                            lesson_learned = Some(remediation);
                            timeline.push(WorkflowTimelineEvent {
                                timestamp: Utc::now().format("%H:%M:%S").to_string(),
                                level: "warn".to_string(),
                                message: format!(
                                    "⚡ [Harness² Self-Healing] Đã tự khắc phục lệnh Shell cho Tool Node '{}'!",
                                    node_label
                                ),
                            });
                        }
                        break;
                    } else {
                        let err_msg = format!("Exit code {}: {}", exit_code, stderr_str);
                        node_error = Some(err_msg.clone());

                        if tool_attempt < max_tool_attempts && self_healing_enabled {
                            if let Some(refined_cmd) = HarnessSelfHealer::propose_tool_refinement(
                                &current_cmd,
                                &stderr_str,
                                exit_code,
                            ) {
                                timeline.push(WorkflowTimelineEvent {
                                    timestamp: Utc::now().format("%H:%M:%S").to_string(),
                                    level: "warn".to_string(),
                                    message: format!(
                                        "🔄 [Harness² Propose-Probe] Tự động sửa lệnh Tool '{}' -> '{}'",
                                        current_cmd, refined_cmd
                                    ),
                                });
                                current_cmd = refined_cmd;
                                tool_attempt += 1;
                                continue;
                            }
                        }
                        status = "failed".to_string();
                        break;
                    }
                }
                Err(e) => {
                    status = "failed".to_string();
                    node_error = Some(format!("Lỗi thực thi lệnh tool: {}", e));
                    break;
                }
            }
        }
    }

    NodeExecutionOutcome {
        node_output,
        node_error,
        engine_used: None,
        status,
        healed,
        healing_attempts,
        lesson_learned,
    }
}

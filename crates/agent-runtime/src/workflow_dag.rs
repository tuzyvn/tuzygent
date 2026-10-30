use crate::harness::{HarnessMemoryStore, HarnessSelfHealer};
use agent_engine::api::OpenAiCompatibleEngine;
use agent_engine::cli::{AntigravityEngine, ClaudeCodeEngine};
use agent_engine::{ComputeEngine, EngineInput};
use anyhow::{anyhow, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowNodeData {
    pub label: Option<String>,
    #[serde(rename = "personaId")]
    pub persona_id: Option<String>,
    #[serde(rename = "personaName")]
    pub persona_name: Option<String>,
    pub engine: Option<String>,
    pub model: Option<String>,
    pub role: Option<String>,
    #[serde(rename = "promptTemplate")]
    pub prompt_template: Option<String>,
    #[serde(rename = "triggerType")]
    pub trigger_type: Option<String>,
    pub endpoint: Option<String>,
    #[serde(rename = "cronExpr")]
    pub cron_expr: Option<String>,
    pub expression: Option<String>,
    #[serde(rename = "requiredRole")]
    pub required_role: Option<String>,
    pub command: Option<String>,
    #[serde(rename = "subWorkflowId")]
    pub sub_workflow_id: Option<String>,
    #[serde(rename = "subWorkflowName")]
    pub sub_workflow_name: Option<String>,
    #[serde(rename = "enableSelfHealing")]
    pub enable_self_healing: Option<bool>,
    #[serde(rename = "maxHealingAttempts")]
    pub max_healing_attempts: Option<u32>,
    pub healed: Option<bool>,
    #[serde(rename = "healingAttempts")]
    pub healing_attempts: Option<u32>,
    #[serde(rename = "lessonLearned")]
    pub lesson_learned: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowNode {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: String,
    pub data: WorkflowNodeData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    #[serde(rename = "sourceHandle")]
    pub source_handle: Option<String>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub version: Option<String>,
    pub nodes: Vec<WorkflowNode>,
    pub edges: Vec<WorkflowEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowNodeExecutionResult {
    pub node_id: String,
    pub node_name: String,
    pub node_type: String,
    pub status: String,
    pub started_at: String,
    pub finished_at: String,
    pub execution_ms: u128,
    pub engine_used: Option<String>,
    pub input: serde_json::Value,
    pub output: serde_json::Value,
    pub error: Option<String>,
    pub healed: Option<bool>,
    pub healing_attempts: Option<u32>,
    pub lesson_learned: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRunReport {
    pub run_id: String,
    pub workflow_id: String,
    pub workflow_name: String,
    pub workflow_version: String,
    pub triggered_by: String,
    pub trigger_type: String,
    pub status: String,
    pub started_at: String,
    pub completed_at: String,
    pub duration_ms: u128,
    pub total_tokens_used: u64,
    pub node_results: HashMap<String, WorkflowNodeExecutionResult>,
    pub timeline: Vec<WorkflowTimelineEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowTimelineEvent {
    pub timestamp: String,
    pub level: String,
    pub message: String,
}

pub struct WorkflowOrchestrator {
    workspace_dir: String,
    _personas_dir: String,
    harness_memory: HarnessMemoryStore,
}

impl WorkflowOrchestrator {
    pub fn new(workspace_dir: &str, personas_dir: &str) -> Self {
        Self {
            workspace_dir: workspace_dir.to_string(),
            _personas_dir: personas_dir.to_string(),
            harness_memory: HarnessMemoryStore::new(workspace_dir),
        }
    }

    fn resolve_engine(
        &self,
        engine_name: Option<&str>,
        model: Option<&str>,
    ) -> Arc<dyn ComputeEngine> {
        let name = engine_name.unwrap_or("agy").to_lowercase();
        // Giá trị default cho model nếu không được chỉ định
        let resolved_model = match model {
            Some(m) if !m.trim().is_empty() => Some(m.to_string()),
            _ => match name.as_str() {
                "agy" => Some("gemini-3.8-flash-high".to_string()),
                "claude" | "claude-code" => Some("claude-sonnet-5-5-high".to_string()),
                "local" | "omniroute" => Some("local".to_string()),
                _ => None,
            },
        };

        if name == "claude" || name == "claude-code" {
            Arc::new(ClaudeCodeEngine::new(None, resolved_model))
        } else if name == "local" || name == "omniroute" {
            Arc::new(OpenAiCompatibleEngine::new(
                "http://127.0.0.1:20128/v1",
                "omniroute",
                resolved_model.as_deref().unwrap_or("local"),
            ))
        } else {
            Arc::new(AntigravityEngine::new(None, resolved_model))
        }
    }

    pub async fn load_workflow_from_catalog(
        &self,
        workflow_id: &str,
    ) -> Result<WorkflowDefinition> {
        let catalog_path = std::path::Path::new(&self.workspace_dir)
            .join("workflows")
            .join("catalog.json");
        let data = tokio::fs::read(&catalog_path)
            .await
            .map_err(|e| anyhow!("Không đọc được file catalog tại {:?}: {}", catalog_path, e))?;
        let workflows: Vec<WorkflowDefinition> =
            serde_json::from_slice(&data).map_err(|e| anyhow!("Lỗi parse JSON catalog: {}", e))?;
        workflows
            .into_iter()
            .find(|w| w.id == workflow_id)
            .ok_or_else(|| {
                anyhow!(
                    "Không tìm thấy Sub-Workflow có ID '{}' trong catalog",
                    workflow_id
                )
            })
    }

    pub async fn execute_workflow(
        &self,
        workflow: &WorkflowDefinition,
        trigger_payload: Option<serde_json::Value>,
        triggered_by: &str,
    ) -> Result<WorkflowRunReport> {
        self.execute_workflow_internal(workflow, trigger_payload, triggered_by, 0)
            .await
    }

    pub fn execute_workflow_internal<'a>(
        &'a self,
        workflow: &'a WorkflowDefinition,
        trigger_payload: Option<serde_json::Value>,
        triggered_by: &'a str,
        depth: usize,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<WorkflowRunReport>> + Send + 'a>>
    {
        Box::pin(async move {
            if depth > 5 {
                return Err(anyhow!(
                    "Vượt quá độ sâu tối đa (5) của Sub-Workflow lồng nhau"
                ));
            }

            let run_id = format!("run-{}", Utc::now().format("%Y%m%d%H%M%S"));
            let start_time = Utc::now();
            let mut timeline: Vec<WorkflowTimelineEvent> = Vec::new();
            let mut node_results: HashMap<String, WorkflowNodeExecutionResult> = HashMap::new();
            let mut context_state: HashMap<String, serde_json::Value> = HashMap::new();

            if let Some(payload) = trigger_payload {
                context_state.insert("trigger".to_string(), payload);
            }

            timeline.push(WorkflowTimelineEvent {
                timestamp: Utc::now().format("%H:%M:%S").to_string(),
                level: "info".to_string(),
                message: format!(
                    "Khởi chạy Workflow '{}' (ID: {}, Depth: {})",
                    workflow.name, workflow.id, depth
                ),
            });

            // Xây dựng DAG (In-degree & Adjacency)
            let mut in_degree: HashMap<String, usize> = HashMap::new();
            let mut adj: HashMap<String, Vec<String>> = HashMap::new();
            let mut node_map: HashMap<String, &WorkflowNode> = HashMap::new();

            for node in &workflow.nodes {
                in_degree.insert(node.id.clone(), 0);
                adj.insert(node.id.clone(), Vec::new());
                node_map.insert(node.id.clone(), node);
            }

            for edge in &workflow.edges {
                if in_degree.contains_key(&edge.target) {
                    *in_degree.get_mut(&edge.target).unwrap() += 1;
                    adj.get_mut(&edge.source).unwrap().push(edge.target.clone());
                }
            }

            // Hàng đợi Topological sort
            let mut queue: VecDeque<String> = VecDeque::new();
            for (id, deg) in &in_degree {
                if *deg == 0 {
                    queue.push_back(id.clone());
                }
            }

            // Theo dõi phiên chạy của Agent trước đó để chuyển giao ngữ cảnh chéo engine (Cross-Engine Handoff)
            let mut last_agent_execution: Option<(String, String)> = None;

            while let Some(current_id) = queue.pop_front() {
                let node = match node_map.get(&current_id) {
                    Some(n) => *n,
                    None => continue,
                };

                let node_label = node
                    .data
                    .label
                    .clone()
                    .unwrap_or_else(|| current_id.clone());
                let node_start = Utc::now();

                timeline.push(WorkflowTimelineEvent {
                    timestamp: node_start.format("%H:%M:%S").to_string(),
                    level: "info".to_string(),
                    message: format!("Thực thi Node '{}' ({})", node_label, node.node_type),
                });

                let mut node_output = serde_json::json!({});
                let mut node_error: Option<String> = None;
                let mut engine_used: Option<String> = None;
                let mut status = "completed".to_string();
                let mut healed: Option<bool> = None;
                let mut healing_attempts: Option<u32> = None;
                let mut lesson_learned: Option<String> = None;

                match node.node_type.as_str() {
                    "triggerNode" => {
                        node_output = serde_json::json!({
                            "triggered": true,
                            "type": node.data.trigger_type.clone().unwrap_or_else(|| "manual".to_string()),
                            "timestamp": Utc::now().to_rfc3339()
                        });
                    }
                    "agentNode" => {
                        let engine_name = node.data.engine.as_deref().unwrap_or("agy");
                        let model_name =
                            node.data.model.as_deref().filter(|m| !m.trim().is_empty());
                        let engine = self.resolve_engine(Some(engine_name), model_name);
                        engine_used = Some(match model_name {
                            Some(m) => format!("{} ({})", engine_name, m),
                            None => engine_name.to_string(),
                        });

                        let persona_identifier =
                            node.data.persona_id.as_deref().unwrap_or("agent-worker");
                        let relevant_lessons = self
                            .harness_memory
                            .find_relevant_lessons("agentNode", persona_identifier)
                            .await;

                        let context_json =
                            serde_json::to_string_pretty(&context_state).unwrap_or_default();
                        let base_prompt = if let Some(tmpl) = &node.data.prompt_template {
                            if !tmpl.trim().is_empty() {
                                if tmpl.contains("{{input}}") {
                                    tmpl.replace("{{input}}", &context_json)
                                } else {
                                    format!("{}\n\n[Context Input]:\n{}", tmpl, context_json)
                                }
                            } else {
                                format!(
                                    "Vai trò của bạn: {}\nNhiệm vụ: Phân tích và xử lý trạng thái workflow hiện tại:\n{}\nHãy đưa ra kết quả xử lý chi tiết.",
                                    node.data.role.as_deref().unwrap_or("Autonomous Agent"),
                                    context_json
                                )
                            }
                        } else {
                            format!(
                                "Vai trò của bạn: {}\nNhiệm vụ: Phân tích và xử lý trạng thái workflow hiện tại:\n{}\nHãy đưa ra kết quả xử lý chi tiết.",
                                node.data.role.as_deref().unwrap_or("Autonomous Agent"),
                                context_json
                            )
                        };

                        let system_context = format!(
                            "Bạn là Persona Agent: {}. Hãy thực thi nhiệm vụ một cách chuyên nghiệp và chính xác.",
                            node.data.persona_name.as_deref().unwrap_or("Worker")
                        );

                        let self_healing_enabled = node.data.enable_self_healing.unwrap_or(true);
                        let max_healing_attempts = node.data.max_healing_attempts.unwrap_or(2);

                        // Cầu nối Cross-Engine Handoff Memory (Cindy-style)
                        let handoff_bridge = if let Some((prev_engine, prev_summary)) =
                            &last_agent_execution
                        {
                            if prev_engine != engine_name {
                                timeline.push(WorkflowTimelineEvent {
                                    timestamp: Utc::now().format("%H:%M:%S").to_string(),
                                    level: "info".to_string(),
                                    message: format!("🔄 [Cross-Engine Handoff] Bàn giao ngữ cảnh bộ nhớ từ Engine '{}' sang '{}'", prev_engine, engine_name),
                                });
                                format!(
                                    "\n\n======================================================\n\
                                    🔄 [Cross-Engine Handoff Context (Bàn giao từ {})]:\n\
                                    Agent tiền nhiệm đã thực thi và tạo ra kết quả sau:\n{}\n\
                                    Hãy kế thừa thông tin này để tiếp nối công việc một cách nhất quán.\n\
                                    ======================================================\n",
                                    prev_engine, prev_summary
                                )
                            } else {
                                String::new()
                            }
                        } else {
                            String::new()
                        };

                        let mut current_prompt = format!("{}{}", base_prompt, handoff_bridge);
                        let mut attempt = 0;
                        let mut execution_succeeded = false;

                        while attempt <= max_healing_attempts {
                            let input = EngineInput {
                                prompt_or_command: current_prompt.clone(),
                                system_context: system_context.clone(),
                                working_dir: self.workspace_dir.clone(),
                                timeout_seconds: 60,
                                on_stream_chunk: None,
                            };

                            let timeout_dur = std::time::Duration::from_secs(input.timeout_seconds);
                            match tokio::time::timeout(timeout_dur, engine.execute(&input)).await {
                                Ok(Ok(out)) => {
                                    let stdout_trimmed = out.stdout_content.trim().to_string();
                                    if out.exit_code == 0 && !stdout_trimmed.is_empty() {
                                        node_output = serde_json::json!({
                                            "stdout": stdout_trimmed,
                                            "exit_code": out.exit_code
                                        });
                                        execution_succeeded = true;
                                        if attempt > 0 {
                                            healed = Some(true);
                                            healing_attempts = Some(attempt);
                                            let remediation = "Điều chỉnh prompt theo phản hồi lỗi và tuân thủ định dạng output".to_string();
                                            let _ = self
                                                .harness_memory
                                                .record_lesson(
                                                    "agentNode",
                                                    persona_identifier,
                                                    &node_error.clone().unwrap_or_else(|| {
                                                        "Exit failure".to_string()
                                                    }),
                                                    &remediation,
                                                )
                                                .await;
                                            lesson_learned = Some(remediation);
                                            timeline.push(WorkflowTimelineEvent {
                                                timestamp: Utc::now().format("%H:%M:%S").to_string(),
                                                level: "warn".to_string(),
                                                message: format!("⚡ [Harness² Self-Healing] Đã tự phục hồi Node '{}' thành công ở lần thử {}/{}!", node_label, attempt, max_healing_attempts),
                                            });
                                        }
                                        node_error = None;
                                        status = "completed".to_string();
                                        last_agent_execution =
                                            Some((engine_name.to_string(), stdout_trimmed.clone()));
                                        break;
                                    } else {
                                        let err_desc = if out.exit_code != 0 {
                                            format!(
                                                "Agent engine kết thúc với exit_code {}",
                                                out.exit_code
                                            )
                                        } else {
                                            "Agent output rỗng hoặc không có dữ liệu trả về"
                                                .to_string()
                                        };
                                        node_error = Some(err_desc.clone());
                                    }
                                }
                                Ok(Err(e)) => {
                                    node_error = Some(e.to_string());
                                }
                                Err(_) => {
                                    node_error = Some(format!(
                                        "Engine execution timed out after {}s",
                                        input.timeout_seconds
                                    ));
                                }
                            }

                            attempt += 1;
                            if attempt <= max_healing_attempts && self_healing_enabled {
                                let err_msg = node_error.as_deref().unwrap_or("Không xác định");
                                timeline.push(WorkflowTimelineEvent {
                                    timestamp: Utc::now().format("%H:%M:%S").to_string(),
                                    level: "warn".to_string(),
                                    message: format!("🔄 [Harness² Propose-Probe] Kích hoạt Self-Healing lần {}/{} cho Node '{}' do lỗi: {}", attempt, max_healing_attempts, node_label, err_msg),
                                });
                                current_prompt = HarnessSelfHealer::propose_agent_refinement(
                                    &base_prompt,
                                    err_msg,
                                    attempt,
                                    max_healing_attempts,
                                    &relevant_lessons,
                                );
                            } else {
                                break;
                            }
                        }

                        if !execution_succeeded {
                            status = "failed".to_string();
                        }
                    }
                    "toolNode" => {
                        if let Some(cmd_str) = &node.data.command {
                            let self_healing_enabled =
                                node.data.enable_self_healing.unwrap_or(true);
                            let mut current_cmd = cmd_str.clone();
                            let mut tool_attempt = 0;
                            let max_tool_attempts = 1;

                            loop {
                                let mut cmd = Command::new("bash");
                                cmd.arg("-c").arg(&current_cmd);
                                cmd.current_dir(&self.workspace_dir);
                                match cmd.output().await {
                                    Ok(output) => {
                                        let stdout_str = String::from_utf8_lossy(&output.stdout)
                                            .trim()
                                            .to_string();
                                        let stderr_str = String::from_utf8_lossy(&output.stderr)
                                            .trim()
                                            .to_string();
                                        let exit_code = output.status.code().unwrap_or(-1);

                                        let parsed_json: serde_json::Value =
                                            serde_json::from_str(&stdout_str).unwrap_or_else(
                                                |_| serde_json::json!({ "raw": stdout_str }),
                                            );

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
                                                let remediation = format!(
                                                    "Điều chỉnh lệnh thực thi sang: {}",
                                                    current_cmd
                                                );
                                                let _ = self
                                                    .harness_memory
                                                    .record_lesson(
                                                        "toolNode",
                                                        &current_id,
                                                        "Command execution failure",
                                                        &remediation,
                                                    )
                                                    .await;
                                                lesson_learned = Some(remediation);
                                                timeline.push(WorkflowTimelineEvent {
                                                    timestamp: Utc::now().format("%H:%M:%S").to_string(),
                                                    level: "warn".to_string(),
                                                    message: format!("⚡ [Harness² Self-Healing] Đã tự khắc phục lệnh Shell cho Tool Node '{}'!", node_label),
                                                });
                                            }
                                            break;
                                        } else {
                                            let err_msg =
                                                format!("Exit code {}: {}", exit_code, stderr_str);
                                            node_error = Some(err_msg.clone());

                                            if tool_attempt < max_tool_attempts
                                                && self_healing_enabled
                                            {
                                                if let Some(refined_cmd) =
                                                    HarnessSelfHealer::propose_tool_refinement(
                                                        &current_cmd,
                                                        &stderr_str,
                                                        exit_code,
                                                    )
                                                {
                                                    timeline.push(WorkflowTimelineEvent {
                                                        timestamp: Utc::now().format("%H:%M:%S").to_string(),
                                                        level: "warn".to_string(),
                                                        message: format!("🔄 [Harness² Propose-Probe] Tự động sửa lệnh Tool '{}' -> '{}'", current_cmd, refined_cmd),
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
                    }
                    "approvalNode" => {
                        node_output = serde_json::json!({
                            "decision": "Approved",
                            "approver": "System Auto-Pilot Guard",
                            "note": "Điều kiện guardrail an toàn được thỏa mãn"
                        });
                    }
                    "routerNode" => {
                        node_output = serde_json::json!({
                            "route_selected": "branch-a",
                            "condition": node.data.expression.clone().unwrap_or_default()
                        });
                    }
                    "subWorkflowNode" | "workflowNode" => {
                        let sub_id = node
                            .data
                            .sub_workflow_id
                            .as_deref()
                            .or_else(|| node.data.endpoint.as_deref())
                            .unwrap_or("");

                        if sub_id.is_empty() {
                            status = "failed".to_string();
                            node_error = Some(
                                "Node Sub-Workflow chưa được gán subWorkflowId hợp lệ".to_string(),
                            );
                        } else {
                            timeline.push(WorkflowTimelineEvent {
                                timestamp: Utc::now().format("%H:%M:%S").to_string(),
                                level: "info".to_string(),
                                message: format!(
                                    "🔀 Đang lồng ghép Sub-Workflow: '{}' (Độ sâu: {})",
                                    sub_id,
                                    depth + 1
                                ),
                            });

                            match self.load_workflow_from_catalog(sub_id).await {
                                Ok(child_wf) => {
                                    let child_payload = Some(serde_json::json!({
                                        "parent_context": context_state.clone(),
                                        "parent_workflow_id": workflow.id.clone(),
                                        "parent_node_id": current_id.clone()
                                    }));

                                    let caller_desc = format!(
                                        "Parent Workflow '{}' (Node: {})",
                                        workflow.name, node_label
                                    );
                                    match self
                                        .execute_workflow_internal(
                                            &child_wf,
                                            child_payload,
                                            &caller_desc,
                                            depth + 1,
                                        )
                                        .await
                                    {
                                        Ok(child_report) => {
                                            for event in &child_report.timeline {
                                                timeline.push(WorkflowTimelineEvent {
                                                    timestamp: event.timestamp.clone(),
                                                    level: event.level.clone(),
                                                    message: format!(
                                                        "[SubWF: {}] {}",
                                                        child_wf.name, event.message
                                                    ),
                                                });
                                            }

                                            let is_success = child_report
                                                .status
                                                .eq_ignore_ascii_case("completed");
                                            status = if is_success {
                                                "completed".to_string()
                                            } else {
                                                "failed".to_string()
                                            };
                                            if !is_success {
                                                node_error = Some(format!(
                                                    "Sub-Workflow '{}' kết thúc với trạng thái: {}",
                                                    child_wf.name, child_report.status
                                                ));
                                            }

                                            node_output = serde_json::json!({
                                                "sub_workflow_id": child_wf.id,
                                                "sub_workflow_name": child_wf.name,
                                                "sub_run_id": child_report.run_id,
                                                "status": child_report.status,
                                                "duration_ms": child_report.duration_ms,
                                                "nodes_executed": child_report.node_results.len(),
                                                "node_results": child_report.node_results
                                            });
                                        }
                                        Err(e) => {
                                            status = "failed".to_string();
                                            node_error = Some(format!(
                                                "Lỗi thực thi Sub-Workflow '{}': {}",
                                                sub_id, e
                                            ));
                                        }
                                    }
                                }
                                Err(e) => {
                                    status = "failed".to_string();
                                    node_error = Some(format!(
                                        "Không thể nạp Sub-Workflow '{}': {}",
                                        sub_id, e
                                    ));
                                }
                            }
                        }
                    }
                    _ => {
                        node_output = serde_json::json!({ "skipped": true });
                    }
                }

                let node_finish = Utc::now();
                let duration_ms = (node_finish - node_start).num_milliseconds().max(0) as u128;

                // Lưu kết quả node vào context chung
                context_state.insert(current_id.clone(), node_output.clone());

                node_results.insert(
                    current_id.clone(),
                    WorkflowNodeExecutionResult {
                        node_id: current_id.clone(),
                        node_name: node_label.clone(),
                        node_type: node.node_type.clone(),
                        status: status.clone(),
                        started_at: node_start.to_rfc3339(),
                        finished_at: node_finish.to_rfc3339(),
                        execution_ms: duration_ms,
                        engine_used,
                        input: serde_json::json!({ "context_keys": context_state.keys().collect::<Vec<_>>() }),
                        output: node_output,
                        error: node_error,
                        healed,
                        healing_attempts,
                        lesson_learned,
                    },
                );

                // Mở khóa các node tiếp theo
                if let Some(neighbors) = adj.get(&current_id) {
                    for neighbor in neighbors {
                        if let Some(deg) = in_degree.get_mut(neighbor) {
                            *deg -= 1;
                            if *deg == 0 {
                                queue.push_back(neighbor.clone());
                            }
                        }
                    }
                }
            }

            let finish_time = Utc::now();
            let total_duration = (finish_time - start_time).num_milliseconds().max(0) as u128;

            timeline.push(WorkflowTimelineEvent {
                timestamp: finish_time.format("%H:%M:%S").to_string(),
                level: "info".to_string(),
                message: format!(
                    "Workflow '{}' hoàn tất trong {}ms",
                    workflow.name, total_duration
                ),
            });

            Ok(WorkflowRunReport {
                run_id,
                workflow_id: workflow.id.clone(),
                workflow_name: workflow.name.clone(),
                workflow_version: workflow
                    .version
                    .clone()
                    .unwrap_or_else(|| "1.0.0".to_string()),
                triggered_by: triggered_by.to_string(),
                trigger_type: "DAG_Orchestrator".to_string(),
                status: "Completed".to_string(),
                started_at: start_time.to_rfc3339(),
                completed_at: finish_time.to_rfc3339(),
                duration_ms: total_duration,
                total_tokens_used: 1850,
                node_results,
                timeline,
            })
        })
    }
}

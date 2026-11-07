use agent_engine::EngineInput;
use anyhow::Result;
use chrono::Utc;

use super::types::{NodeExecutionContext, NodeExecutionOutcome};
use crate::harness::HarnessSelfHealer;
use crate::workflow_dag::orchestrator::WorkflowOrchestrator;
use crate::workflow_dag::types::{WorkflowNode, WorkflowTimelineEvent};

fn default_agent_prompt(node: &WorkflowNode, context_json: &str) -> String {
    format!(
        "Vai trò của bạn: {}\nNhiệm vụ: Phân tích và xử lý trạng thái workflow hiện tại:\n{}\nHãy đưa ra kết quả xử lý chi tiết.",
        node.data.role.as_deref().unwrap_or("Autonomous Agent"), context_json
    )
}

pub async fn execute_agent_node(
    orchestrator: &WorkflowOrchestrator,
    node: &WorkflowNode,
    node_label: &str,
    ctx: &mut NodeExecutionContext<'_>,
) -> Result<NodeExecutionOutcome> {
    let engine_config = agent_engine::EnginesConfig::load_hierarchical(Some(std::path::Path::new(
        &orchestrator.workspace_dir,
    )));
    let engine_name = node
        .data
        .engine
        .as_deref()
        .unwrap_or(&engine_config.defaults.engine);
    let model_name = node.data.model.as_deref().filter(|m| !m.trim().is_empty());
    let engine = orchestrator.resolve_engine(Some(engine_name), model_name)?;
    let execution =
        crate::execution::ExecutionService::from_workspace(&orchestrator.workspace_dir)?;
    let engine_used = Some(match model_name {
        Some(m) => format!("{} ({})", engine_name, m),
        None => engine_name.to_string(),
    });

    let persona_identifier = node.data.persona_id.as_deref().unwrap_or("agent-worker");
    let relevant_lessons = orchestrator
        .harness_memory
        .find_relevant_lessons("agentNode", persona_identifier)
        .await;

    let context_json = serde_json::to_string_pretty(&ctx.context_state).unwrap_or_default();
    let base_prompt = match &node.data.prompt_template {
        Some(tmpl) if !tmpl.trim().is_empty() => {
            if tmpl.contains("{{input}}") {
                tmpl.replace("{{input}}", &context_json)
            } else {
                format!("{}\n\n[Context Input]:\n{}", tmpl, context_json)
            }
        }
        _ => default_agent_prompt(node, &context_json),
    };

    let system_context = format!(
        "Bạn là Persona Agent: {}. Hãy thực thi nhiệm vụ một cách chuyên nghiệp và chính xác.",
        node.data.persona_name.as_deref().unwrap_or("Worker")
    );
    let self_healing_enabled = node.data.enable_self_healing.unwrap_or(true);
    let max_healing_attempts = node.data.max_healing_attempts.unwrap_or(2);

    // Cầu nối Cross-Engine Handoff Memory (Cindy-style)
    let handoff_bridge = match ctx.last_agent_execution {
        Some((prev_engine, prev_summary)) if prev_engine != engine_name => {
            ctx.timeline.push(WorkflowTimelineEvent {
                timestamp: Utc::now().format("%H:%M:%S").to_string(),
                level: "info".to_string(),
                message: format!(
                    "🔄 [Cross-Engine Handoff] Bàn giao ngữ cảnh bộ nhớ từ Engine '{}' sang '{}'",
                    prev_engine, engine_name
                ),
            });
            format!(
                "\n\n======================================================\n\
                🔄 [Cross-Engine Handoff Context (Bàn giao từ {})]:\nAgent tiền nhiệm đã thực thi và tạo ra kết quả sau:\n{}\n\
                Hãy kế thừa thông tin này để tiếp nối công việc một cách nhất quán.\n======================================================\n",
                prev_engine, prev_summary
            )
        }
        _ => String::new(),
    };

    let mut current_prompt = format!("{}{}", base_prompt, handoff_bridge);
    let (mut attempt, mut execution_succeeded) = (0, false);
    let (mut node_output, mut node_error) = (serde_json::json!({}), None);
    let (mut healed, mut healing_attempts, mut lesson_learned) = (None, None, None);
    let mut status = "completed".to_string();

    while attempt <= max_healing_attempts {
        let input = EngineInput {
            prompt_or_command: current_prompt.clone(),
            system_context: system_context.clone(),
            working_dir: orchestrator.workspace_dir.clone(),
            timeout_seconds: 60,
            on_stream_chunk: None,
            command_sandbox: None,
            on_event: None,
            temperature: None,
            max_tokens: None,
            max_output_bytes: None,
        };

        match execution.execute(engine.clone(), &input).await {
            Ok(out) => {
                let stdout_trimmed = out.stdout_content.trim().to_string();
                if out.exit_code == 0 && !stdout_trimmed.is_empty() {
                    node_output =
                        serde_json::json!({ "stdout": stdout_trimmed, "exit_code": out.exit_code });
                    execution_succeeded = true;
                    if attempt > 0 {
                        healed = Some(true);
                        healing_attempts = Some(attempt);
                        let remediation =
                            "Điều chỉnh prompt theo phản hồi lỗi và tuân thủ định dạng output"
                                .to_string();
                        let _ = orchestrator
                            .harness_memory
                            .record_lesson(
                                "agentNode",
                                persona_identifier,
                                &node_error
                                    .clone()
                                    .unwrap_or_else(|| "Exit failure".to_string()),
                                &remediation,
                            )
                            .await;
                        lesson_learned = Some(remediation);
                        ctx.timeline.push(WorkflowTimelineEvent {
                            timestamp: Utc::now().format("%H:%M:%S").to_string(),
                            level: "warn".to_string(),
                            message: format!("⚡ [Harness² Self-Healing] Đã tự phục hồi Node '{}' thành công ở lần thử {}/{}!", node_label, attempt, max_healing_attempts),
                        });
                    }
                    node_error = None;
                    status = "completed".to_string();
                    *ctx.last_agent_execution = Some((engine_name.to_string(), stdout_trimmed));
                    break;
                } else {
                    node_error = Some(if out.exit_code != 0 {
                        format!("Agent engine kết thúc với exit_code {}", out.exit_code)
                    } else {
                        "Agent output rỗng hoặc không có dữ liệu trả về".to_string()
                    });
                }
            }
            Err(ref e)
                if e.is_auth_or_quota()
                    || matches!(
                        e,
                        agent_engine::EngineError::PolicyRejected(_)
                            | agent_engine::EngineError::CliNotFound(_)
                    ) =>
            {
                node_error = Some(e.to_string());
                status = "failed".to_string();
                break;
            }
            Err(e) => node_error = Some(e.to_string()),
        }

        attempt += 1;
        if attempt <= max_healing_attempts && self_healing_enabled {
            let err_msg = node_error.as_deref().unwrap_or("Không xác định");
            ctx.timeline.push(WorkflowTimelineEvent {
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

    Ok(NodeExecutionOutcome {
        node_output,
        node_error,
        engine_used,
        status,
        healed,
        healing_attempts,
        lesson_learned,
    })
}

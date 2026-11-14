use super::types::{
    CoordinateBotsRequest, CoordinationContext, TeammateTurnResult,
};
use super::KernelOperator;
use agent_domain::{wrap_with_provenance, ProvenanceAuthority, RigSeatConfig};
use agent_engine::EngineInput;
use std::collections::HashMap;

pub(super) trait CoordinatePromptOps {
    fn norm_target(raw: &str) -> String;
    fn evaluate_done_when(done_when: Option<&str>, exit_code: i32, output: &str) -> bool;
    fn build_env_metadata(
        seat: &RigSeatConfig,
        caller_id: &str,
        child_ctx: &CoordinationContext,
        consult: bool,
    ) -> HashMap<String, String>;
    fn build_system_context(env: &HashMap<String, String>) -> String;
    fn build_teammate_prompt(
        seat: &RigSeatConfig,
        req: &CoordinateBotsRequest,
        child_ctx: &CoordinationContext,
        system_prompt: Option<&str>,
    ) -> String;
    fn build_resume_prompt(req: &CoordinateBotsRequest, results: &[TeammateTurnResult]) -> String;
    fn make_engine_input(
        prompt: String,
        system_context: String,
        working_dir: &str,
    ) -> EngineInput;
}

impl CoordinatePromptOps for KernelOperator {
    fn norm_target(raw: &str) -> String {
        raw.trim().trim_start_matches('@').to_ascii_lowercase()
    }

    fn evaluate_done_when(done_when: Option<&str>, exit_code: i32, output: &str) -> bool {
        if exit_code != 0 || output.trim().is_empty() {
            return false;
        }
        let Some(criteria) = done_when else {
            return true;
        };
        let output_lower = output.to_ascii_lowercase();
        criteria.split([';', ',', '\n']).all(|segment| {
            let lower = segment.trim().to_ascii_lowercase();
            match lower.strip_prefix("must_contain:").map(str::trim) {
                Some(token) if !token.is_empty() => output_lower.contains(token),
                _ => true,
            }
        })
    }

    fn build_env_metadata(
        seat: &RigSeatConfig,
        caller_id: &str,
        child_ctx: &CoordinationContext,
        consult: bool,
    ) -> HashMap<String, String> {
        HashMap::from([
            ("TUZYGENT_BOT_ID".to_string(), seat.seat_id.clone()),
            ("TUZYGENT_THREAD_ID".to_string(), child_ctx.thread_id.clone()),
            ("TUZYGENT_TURN_DEPTH".to_string(), child_ctx.depth.to_string()),
            (
                "TUZYGENT_ASSIGNED_BY".to_string(),
                caller_id.trim().trim_start_matches('@').to_string(),
            ),
            (
                "TUZYGENT_CONSULT_ONLY".to_string(),
                if consult { "1" } else { "0" }.to_string(),
            ),
        ])
    }

    fn build_system_context(env: &HashMap<String, String>) -> String {
        [
            "TUZYGENT_BOT_ID",
            "TUZYGENT_THREAD_ID",
            "TUZYGENT_TURN_DEPTH",
            "TUZYGENT_ASSIGNED_BY",
            "TUZYGENT_CONSULT_ONLY",
        ]
        .iter()
        .map(|k| format!("{k}={}", env.get(*k).map(String::as_str).unwrap_or_default()))
        .collect::<Vec<_>>()
        .join(" ")
    }

    fn build_teammate_prompt(
        seat: &RigSeatConfig,
        req: &CoordinateBotsRequest,
        child_ctx: &CoordinationContext,
        system_prompt: Option<&str>,
    ) -> String {
        let mode_banner = if req.consult {
            "[CONSULTATION ONLY - DO NOT MODIFY FILES]\nAnswer directly from your domain expertise and read-only inspection."
        } else if req.rework {
            "[REWORK REQUEST]\nPrevious output did not satisfy verification criteria. Apply concrete corrections."
        } else {
            "[TEAMMATE HANDOFF]\nExecute your assigned slice cleanly and report concrete results."
        };
        let done_clause = req
            .done_when
            .as_deref()
            .map(|d| format!("\nDone when: {d}"))
            .unwrap_or_default();
        let prompt_clause = system_prompt
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|sp| format!("\n\nSystem Prompt:\n{sp}"))
            .unwrap_or_default();
        let ancestors = if child_ctx.ancestors.is_empty() {
            "none".to_string()
        } else {
            child_ctx.ancestors.join(", ")
        };
        format!(
            "{mode_banner}\nTarget Seat: {} (Role: {})\nAssigned by: {} (thread: {}, depth: {}/{})\nAnti-cycle rule: Never target {} or ancestors [{ancestors}] in coordinate_bots; end your turn to report back.{done_clause}{prompt_clause}\n\nInstructions:\n{}",
            seat.seat_id,
            seat.role,
            req.caller_id.trim(),
            child_ctx.thread_id,
            child_ctx.depth,
            child_ctx.max_depth,
            req.caller_id.trim(),
            req.instructions.trim()
        )
    }

    fn build_resume_prompt(req: &CoordinateBotsRequest, results: &[TeammateTurnResult]) -> String {
        let mut lines = vec![format!(
            "Teammates completed coordination turn for caller '{}' (consult={}, rework={}).",
            req.caller_id.trim(),
            req.consult,
            req.rework
        )];
        if let Some(ref done_when) = req.done_when {
            lines.push(format!("Verification criteria (done_when): {done_when}"));
        }
        for res in results {
            let wrapped = wrap_with_provenance(
                &res.seat_id,
                "teammate_report",
                ProvenanceAuthority::InformationOnly,
                Some("Verify output against done_when before accepting."),
                &res.output,
            );
            lines.push(format!(
                "\n--- Seat '{}' [status={}, done_when_satisfied={}] ---\n{wrapped}",
                res.seat_id, res.status, res.done_when_satisfied
            ));
        }
        let unmet: Vec<&str> = results
            .iter()
            .filter(|r| !r.done_when_satisfied)
            .map(|r| r.seat_id.as_str())
            .collect();
        if unmet.is_empty() {
            lines.push(
                "\nSynthesize the verified teammate findings and produce the final caller response."
                    .to_string(),
            );
        } else {
            lines.push(format!(
                "\nAction needed: Teammate(s) [{}] did not satisfy done_when; consider rework=true with concrete corrections.",
                unmet.join(", ")
            ));
        }
        lines.join("\n")
    }

    fn make_engine_input(
        prompt: String,
        system_context: String,
        working_dir: &str,
    ) -> EngineInput {
        EngineInput {
            prompt_or_command: prompt,
            system_context,
            working_dir: working_dir.to_string(),
            timeout_seconds: 60,
            on_stream_chunk: None,
            command_sandbox: None,
            on_event: None,
            temperature: None,
            max_tokens: None,
            max_output_bytes: None,
        }
    }
}

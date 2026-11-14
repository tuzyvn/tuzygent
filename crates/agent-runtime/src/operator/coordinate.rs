use super::coordinate_prompts::CoordinatePromptOps;
use super::types::{
    CoordinateBotsRequest, CoordinationBatchResult, CoordinationContext, CoordinationReplyMode,
    RoomTargetInfo, TeammateTurnResult, MAX_COORDINATE_BOTS,
};
use super::KernelOperator;
use crate::bus::InterAgentBus;
use agent_domain::{redact_secrets_in_text, AgentId, InterAgentEvent, RigSeatConfig};
use std::collections::HashSet;

impl KernelOperator {
    /// List available teammate seats in the rig without exposing other threads' transcripts.
    pub fn list_room_targets(&self) -> Vec<RoomTargetInfo> {
        self.spec
            .seats
            .iter()
            .map(|seat| RoomTargetInfo {
                seat_id: seat.seat_id.clone(),
                name: seat.name.clone().unwrap_or_else(|| seat.seat_id.clone()),
                role: seat.role.clone(),
                engine: seat.engine.clone().unwrap_or_else(|| "default".to_string()),
                capabilities: seat.capabilities.clone(),
            })
            .collect()
    }

    /// Resolve a teammate seat by exact `seat_id` or unique case-insensitive `name` (optional `@` prefix).
    pub fn resolve_teammate_seat(&self, id_or_name: &str) -> Result<&RigSeatConfig, String> {
        let needle = Self::norm_target(id_or_name);
        if needle.is_empty() {
            return Err("Teammate target identifier cannot be empty.".to_string());
        }
        if let Some(exact) = self
            .spec
            .seats
            .iter()
            .find(|s| s.seat_id.eq_ignore_ascii_case(&needle))
        {
            return Ok(exact);
        }
        let matches: Vec<&RigSeatConfig> = self
            .spec
            .seats
            .iter()
            .filter(|s| {
                s.name
                    .as_deref()
                    .is_some_and(|n| Self::norm_target(n) == needle)
            })
            .collect();
        match matches.len() {
            1 => Ok(matches[0]),
            n if n > 1 => Err(format!(
                "Ambiguous teammate name '{}'; use exact seat_id instead.",
                id_or_name.trim()
            )),
            _ => {
                let available: Vec<&str> =
                    self.spec.seats.iter().map(|s| s.seat_id.as_str()).collect();
                Err(format!(
                    "Unknown teammate target '{}'. Available seats: [{}]",
                    id_or_name.trim(),
                    available.join(", ")
                ))
            }
        }
    }

    /// Validate OpenMausBot coordination invariants: 1..=4 bots, no duplicates, depth < max_depth, and anti-cycle ancestor block.
    pub fn validate_coordination_request(
        &self,
        req: &CoordinateBotsRequest,
        ctx: &CoordinationContext,
    ) -> Result<Vec<RigSeatConfig>, String> {
        let caller = req.caller_id.trim();
        if caller.is_empty() {
            return Err("caller_id cannot be empty in coordinate_bots.".to_string());
        }
        if req.instructions.trim().is_empty() {
            return Err("instructions cannot be empty in coordinate_bots.".to_string());
        }
        if req.bot_ids.is_empty() || req.bot_ids.len() > MAX_COORDINATE_BOTS {
            return Err(format!(
                "coordinate_bots requires 1 to {MAX_COORDINATE_BOTS} target bot_ids (got {}).",
                req.bot_ids.len()
            ));
        }
        if ctx.depth >= ctx.max_depth {
            return Err(format!(
                "Maximum coordination depth ({}) reached; finish your turn and report back to your caller.",
                ctx.max_depth
            ));
        }

        let mut blocked = HashSet::new();
        for raw in std::iter::once(caller).chain(ctx.ancestors.iter().map(String::as_str)) {
            let norm = Self::norm_target(raw);
            if !norm.is_empty() {
                blocked.insert(norm);
            }
            if let Ok(seat) = self.resolve_teammate_seat(raw) {
                blocked.insert(Self::norm_target(&seat.seat_id));
                if let Some(ref display) = seat.name {
                    blocked.insert(Self::norm_target(display));
                }
            }
        }

        let mut seen_ids = HashSet::new();
        let mut resolved = Vec::with_capacity(req.bot_ids.len());
        for raw_target in &req.bot_ids {
            if blocked.contains(&Self::norm_target(raw_target)) {
                return Err(format!(
                    "Anti-cycle guard: cannot target caller or ancestor '{}' in coordinate_bots; end your turn to report back to whoever assigned you this work.",
                    raw_target.trim()
                ));
            }
            let seat = self.resolve_teammate_seat(raw_target)?;
            let seat_norm = Self::norm_target(&seat.seat_id);
            let name_blocked = seat
                .name
                .as_deref()
                .is_some_and(|n| blocked.contains(&Self::norm_target(n)));
            if blocked.contains(&seat_norm) || name_blocked {
                return Err(format!(
                    "Anti-cycle guard: cannot target caller or ancestor '{}' in coordinate_bots; end your turn to report back to whoever assigned you this work.",
                    seat.seat_id
                ));
            }
            if !seen_ids.insert(seat_norm) {
                return Err(format!(
                    "Duplicate teammate target '{}' in coordinate_bots batch.",
                    seat.seat_id
                ));
            }
            resolved.push(seat.clone());
        }
        Ok(resolved)
    }

    /// Dispatch a validated OpenMausBot-style handoff batch to 1-4 teammate seats.
    pub async fn coordinate_bots(
        &self,
        req: &CoordinateBotsRequest,
        ctx: &CoordinationContext,
        engine_override: Option<&str>,
    ) -> Result<CoordinationBatchResult, String> {
        let targets = self.validate_coordination_request(req, ctx)?;
        let child_ctx = ctx.child_for(&req.caller_id)?;
        let bus = InterAgentBus::new(64);
        let execution = if req.reply_mode == CoordinationReplyMode::WaitAndResume {
            Some(
                crate::execution::ExecutionService::from_workspace(&self.workspace_dir)
                    .map_err(|e| e.to_string())?,
            )
        } else {
            None
        };

        let mut results = Vec::with_capacity(targets.len());
        for seat in &targets {
            let env_metadata =
                Self::build_env_metadata(seat, &req.caller_id, &child_ctx, req.consult);
            let _ = bus.publish(InterAgentEvent::new(
                AgentId(req.caller_id.trim().to_string()),
                Some(AgentId(seat.seat_id.clone())),
                "rig.coordinate.assigned",
                serde_json::json!({
                    "thread_id": child_ctx.thread_id,
                    "depth": child_ctx.depth,
                    "consult": req.consult,
                    "rework": req.rework,
                    "reply_mode": req.reply_mode,
                    "env": env_metadata,
                }),
            ));

            if req.reply_mode == CoordinationReplyMode::None {
                results.push(TeammateTurnResult {
                    seat_id: seat.seat_id.clone(),
                    name: seat.name.clone().unwrap_or_else(|| seat.seat_id.clone()),
                    role: seat.role.clone(),
                    status: "notified".to_string(),
                    output: String::new(),
                    done_when_satisfied: true,
                    child_depth: child_ctx.depth,
                    env_metadata,
                });
                continue;
            }

            let engine = self.resolve_engine_with_model(
                seat.engine.as_deref(),
                engine_override,
                seat.model.as_deref(),
            )?;
            let system_prompt = self.seat_system_prompt(&seat.seat_id);
            let input = Self::make_engine_input(
                Self::build_teammate_prompt(seat, req, &child_ctx, system_prompt.as_deref()),
                Self::build_system_context(&env_metadata),
                &self.workspace_dir,
            );
            let (mut status, mut exit_code) = ("Completed".to_string(), 0);
            let raw_output = match execution.as_ref() {
                Some(exec) => match exec.execute(engine, &input).await {
                    Ok(out) => {
                        exit_code = out.exit_code;
                        if out.exit_code != 0 {
                            status = "Failed".to_string();
                        }
                        out.stdout_content
                    }
                    Err(err) => {
                        status = "Failed".to_string();
                        exit_code = 1;
                        err.to_string()
                    }
                },
                None => String::new(),
            };
            let output = redact_secrets_in_text(&raw_output);
            let satisfied = Self::evaluate_done_when(req.done_when.as_deref(), exit_code, &output);
            let topic = if satisfied {
                "rig.coordinate.completed"
            } else {
                "rig.coordinate.rework_needed"
            };
            let _ = bus.publish(InterAgentEvent::new(
                AgentId(seat.seat_id.clone()),
                Some(AgentId(req.caller_id.trim().to_string())),
                topic,
                serde_json::json!({
                    "thread_id": child_ctx.thread_id,
                    "depth": child_ctx.depth,
                    "status": status,
                    "done_when_satisfied": satisfied,
                }),
            ));
            results.push(TeammateTurnResult {
                seat_id: seat.seat_id.clone(),
                name: seat.name.clone().unwrap_or_else(|| seat.seat_id.clone()),
                role: seat.role.clone(),
                status,
                output,
                done_when_satisfied: satisfied,
                child_depth: child_ctx.depth,
                env_metadata,
            });
        }

        let resume_prompt_for_caller = (req.reply_mode == CoordinationReplyMode::WaitAndResume)
            .then(|| Self::build_resume_prompt(req, &results));
        Ok(CoordinationBatchResult {
            rig_id: self.spec.rig.id.clone(),
            thread_id: child_ctx.thread_id,
            caller_id: req.caller_id.trim().to_string(),
            depth: ctx.depth,
            consult: req.consult,
            rework: req.rework,
            reply_mode: req.reply_mode,
            results,
            resume_prompt_for_caller,
            caller_resume_output: None,
        })
    }

    /// Execute `coordinate_bots` and automatically resume the caller with collected teammate replies when `reply_mode == WaitAndResume`.
    pub async fn coordinate_and_resume(
        &self,
        req: &CoordinateBotsRequest,
        ctx: &CoordinationContext,
        engine_override: Option<&str>,
    ) -> Result<CoordinationBatchResult, String> {
        let mut batch = self.coordinate_bots(req, ctx, engine_override).await?;
        let Some(ref resume_prompt) = batch.resume_prompt_for_caller else {
            return Ok(batch);
        };
        let (caller_engine, caller_model) = match self.resolve_teammate_seat(&req.caller_id) {
            Ok(seat) => (seat.engine.as_deref(), seat.model.as_deref()),
            Err(_) => (
                self.spec.operator.engine.as_deref(),
                self.spec.operator.model.as_deref(),
            ),
        };
        let engine =
            self.resolve_engine_with_model(caller_engine, engine_override, caller_model)?;
        let execution = crate::execution::ExecutionService::from_workspace(&self.workspace_dir)
            .map_err(|e| e.to_string())?;
        let input = Self::make_engine_input(
            resume_prompt.clone(),
            format!(
                "Caller Auto-Resume for '{}' (thread: {}, depth: {})",
                batch.caller_id, batch.thread_id, batch.depth
            ),
            &self.workspace_dir,
        );
        let resumed = execution
            .execute(engine, &input)
            .await
            .map_err(|e| e.to_string())?;
        batch.caller_resume_output = Some(redact_secrets_in_text(&resumed.stdout_content));
        Ok(batch)
    }
}

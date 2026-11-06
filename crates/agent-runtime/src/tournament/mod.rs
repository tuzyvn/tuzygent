pub mod judge;

#[cfg(test)]
mod tests;

pub use judge::{AutoJudge, AutoJudgeConfig, CandidateScorecard};

use crate::execution::ExecutionService;
use agent_domain::VerificationSpec;
use agent_engine::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{info, warn};

/// Represents a single engine entrant in a Best-of-N speculative tournament.
#[derive(Clone)]
pub struct TournamentCandidate {
    pub id: String,
    pub engine: Arc<dyn ComputeEngine>,
    pub workspace_override: Option<PathBuf>,
    pub diff_bytes_hint: Option<usize>,
    pub custom_verification_specs: Option<Vec<VerificationSpec>>,
}

impl TournamentCandidate {
    pub fn new(id: impl Into<String>, engine: Arc<dyn ComputeEngine>) -> Self {
        Self {
            id: id.into(),
            engine,
            workspace_override: None,
            diff_bytes_hint: None,
            custom_verification_specs: None,
        }
    }

    pub fn with_workspace_override(mut self, path: impl AsRef<Path>) -> Self {
        self.workspace_override = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn with_diff_bytes_hint(mut self, bytes: usize) -> Self {
        self.diff_bytes_hint = Some(bytes);
        self
    }

    pub fn with_verification_specs(mut self, specs: Vec<VerificationSpec>) -> Self {
        self.custom_verification_specs = Some(specs);
        self
    }
}

/// Final outcome of a Best-of-N speculative tournament execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TournamentOutcome {
    pub winner_id: String,
    pub winner_engine: EngineKind,
    pub winner_output: EngineOutput,
    pub winner_score: f64,
    pub scorecards: Vec<CandidateScorecard>,
    pub used_fallback: bool,
}

/// Speculative Parallel Engine Tournament Executor (`Best-of-N`) with objective `AutoJudge`.
#[derive(Clone)]
pub struct TournamentExecutor {
    workspace_dir: PathBuf,
    candidates: Vec<TournamentCandidate>,
    fallback_engine: Option<Arc<dyn ComputeEngine>>,
    execution_service: ExecutionService,
    verification_specs: Vec<VerificationSpec>,
    judge: AutoJudge,
}

impl TournamentExecutor {
    pub fn new(
        workspace_dir: impl AsRef<Path>,
        engines: Vec<Arc<dyn ComputeEngine>>,
    ) -> Self {
        let candidates = engines
            .into_iter()
            .enumerate()
            .map(|(idx, eng)| {
                let id = format!("{}-{}", eng.engine_kind(), idx);
                TournamentCandidate::new(id, eng)
            })
            .collect();
        Self::with_named_candidates(workspace_dir, candidates)
    }

    pub fn with_named_candidates(
        workspace_dir: impl AsRef<Path>,
        candidates: Vec<TournamentCandidate>,
    ) -> Self {
        let ws = workspace_dir.as_ref().to_path_buf();
        Self {
            execution_service: ExecutionService::new(&ws, None, None),
            workspace_dir: ws,
            candidates,
            fallback_engine: None,
            verification_specs: Vec::new(),
            judge: AutoJudge::default(),
        }
    }

    pub fn with_execution_service(mut self, service: ExecutionService) -> Self {
        self.execution_service = service;
        self
    }

    pub fn with_verification_specs(mut self, specs: Vec<VerificationSpec>) -> Self {
        self.verification_specs = specs;
        self
    }

    pub fn with_judge(mut self, judge: AutoJudge) -> Self {
        self.judge = judge;
        self
    }

    pub fn with_fallback_engine(mut self, fallback: Arc<dyn ComputeEngine>) -> Self {
        self.fallback_engine = Some(fallback);
        self
    }

    pub fn add_candidate(&mut self, candidate: TournamentCandidate) {
        self.candidates.push(candidate);
    }

    pub fn candidates(&self) -> &[TournamentCandidate] {
        &self.candidates
    }

    /// Runs all candidate engines concurrently in parallel, evaluates each candidate via `AutoJudge`
    /// and `TaskVerifier`, selects the highest-scoring valid result, or invokes `fallback_engine`
    /// if all primary candidates fail.
    pub async fn execute_tournament(
        &self,
        input: &EngineInput,
    ) -> Result<TournamentOutcome, EngineError> {
        if self.candidates.is_empty() && self.fallback_engine.is_none() {
            return Err(EngineError::CliNotFound(
                "TournamentExecutor has no registered candidate or fallback engines".into(),
            ));
        }

        info!(
            "🏆 [TournamentExecutor] Starting Best-of-{} parallel tournament",
            self.candidates.len()
        );

        let mut join_set = tokio::task::JoinSet::new();
        for (idx, candidate) in self.candidates.iter().cloned().enumerate() {
            let service = self.execution_service.clone();
            let judge = self.judge.clone();
            let default_specs = self.verification_specs.clone();
            let base_ws = self.workspace_dir.clone();
            let mut candidate_input = input.clone();
            let candidate_ws = candidate
                .workspace_override
                .clone()
                .unwrap_or_else(|| {
                    if !candidate_input.working_dir.is_empty() {
                        PathBuf::from(&candidate_input.working_dir)
                    } else {
                        base_ws
                    }
                });
            candidate_input.working_dir = candidate_ws.to_string_lossy().to_string();

            join_set.spawn(async move {
                let kind = candidate.engine.engine_kind();
                let exec_res = service.execute(candidate.engine.clone(), &candidate_input).await;
                let diff_bytes = candidate.diff_bytes_hint.unwrap_or_else(|| {
                    exec_res
                        .as_ref()
                        .map(|o| o.stdout_content.len())
                        .unwrap_or(0)
                });
                let specs_to_run = candidate
                    .custom_verification_specs
                    .as_deref()
                    .unwrap_or(&default_specs);
                let scorecard = judge
                    .evaluate_with_verifier(
                        &candidate_ws,
                        &candidate.id,
                        kind,
                        &exec_res,
                        specs_to_run,
                        diff_bytes,
                    )
                    .await;
                (idx, scorecard, exec_res)
            });
        }

        let mut evaluated: Vec<(usize, CandidateScorecard, Result<EngineOutput, EngineError>)> =
            Vec::with_capacity(self.candidates.len());

        while let Some(joined) = join_set.join_next().await {
            match joined {
                Ok(entry) => evaluated.push(entry),
                Err(join_err) => {
                    warn!("Tournament candidate task aborted/panicked: {}", join_err);
                }
            }
        }

        evaluated.sort_by(|a, b| {
            b.1.total_score
                .partial_cmp(&a.1.total_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    let dur_a = a.1.execution_duration_ms.unwrap_or(u128::MAX);
                    let dur_b = b.1.execution_duration_ms.unwrap_or(u128::MAX);
                    dur_a.cmp(&dur_b)
                })
                .then_with(|| a.0.cmp(&b.0))
        });

        let mut scorecards: Vec<CandidateScorecard> =
            evaluated.iter().map(|(_, sc, _)| sc.clone()).collect();

        // First priority: highest-scoring candidate with exit_code == 0
        if let Some((_, best_sc, Ok(best_out))) = evaluated
            .iter()
            .find(|(_, sc, res)| res.is_ok() && sc.exit_code == Some(0))
        {
            return Ok(TournamentOutcome {
                winner_id: best_sc.candidate_id.clone(),
                winner_engine: best_sc.engine_kind.clone(),
                winner_output: best_out.clone(),
                winner_score: best_sc.total_score,
                scorecards,
                used_fallback: false,
            });
        }

        // If all primary candidates failed or exited non-zero, try fallback_engine if configured
        if let Some(ref fallback) = self.fallback_engine {
            info!("⚠️ [TournamentExecutor] Primary candidates did not succeed; invoking fallback engine");
            let fb_kind = fallback.engine_kind();
            let fb_res = self.execution_service.execute(fallback.clone(), input).await;
            let fb_diff = fb_res.as_ref().map(|o| o.stdout_content.len()).unwrap_or(0);
            let fb_sc = self
                .judge
                .evaluate_with_verifier(
                    &self.workspace_dir,
                    "fallback",
                    fb_kind.clone(),
                    &fb_res,
                    &self.verification_specs,
                    fb_diff,
                )
                .await;
            let fb_score = fb_sc.total_score;
            scorecards.push(fb_sc);
            if let Ok(fb_out) = fb_res {
                return Ok(TournamentOutcome {
                    winner_id: "fallback".to_string(),
                    winner_engine: fb_kind,
                    winner_output: fb_out,
                    winner_score: fb_score,
                    scorecards,
                    used_fallback: true,
                });
            }
        }

        // Secondary fallback: if any candidate returned Ok(EngineOutput) despite non-zero exit_code
        if let Some((_, best_sc, Ok(best_out))) = evaluated.into_iter().find(|(_, _, res)| res.is_ok()) {
            return Ok(TournamentOutcome {
                winner_id: best_sc.candidate_id,
                winner_engine: best_sc.engine_kind,
                winner_output: best_out,
                winner_score: best_sc.total_score,
                scorecards,
                used_fallback: false,
            });
        }

        let err_summary = scorecards
            .iter()
            .filter_map(|sc| sc.error_message.as_deref())
            .collect::<Vec<_>>()
            .join("; ");
        Err(EngineError::ProcessFailed(format!(
            "All tournament candidates failed: {}",
            if err_summary.is_empty() {
                "no valid output produced"
            } else {
                &err_summary
            }
        )))
    }
}

#[async_trait::async_trait]
impl ComputeEngine for TournamentExecutor {
    async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
        let outcome = self.execute_tournament(input).await?;
        Ok(outcome.winner_output)
    }

    fn engine_kind(&self) -> EngineKind {
        self.candidates
            .first()
            .map(|c| c.engine.engine_kind())
            .or_else(|| self.fallback_engine.as_ref().map(|f| f.engine_kind()))
            .unwrap_or(EngineKind::Antigravity)
    }
}

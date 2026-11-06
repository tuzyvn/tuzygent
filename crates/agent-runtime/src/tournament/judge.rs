use crate::verifier::TaskVerifier;
use agent_domain::{VerificationResult, VerificationSpec};
use agent_engine::{EngineError, EngineKind, EngineOutput};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Configuration weights for objective Auto-Judge candidate evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoJudgeConfig {
    /// Weight multiplied by verification pass rate (0.0..=1.0).
    pub verification_weight: f64,
    /// Bonus awarded when candidate exit_code == 0 (or penalty subtracted when non-zero).
    pub exit_code_zero_bonus: f64,
    /// Bonus awarded when candidate produces non-empty output meeting minimum bytes.
    pub non_empty_output_bonus: f64,
    /// Penalty subtracted per second of execution duration.
    pub latency_penalty_per_sec: f64,
    /// Penalty subtracted per KiB (1024 bytes) of diff / output size.
    pub diff_size_penalty_per_kb: f64,
    /// Minimum output bytes to qualify for non_empty_output_bonus.
    pub min_output_bytes: usize,
}

impl Default for AutoJudgeConfig {
    fn default() -> Self {
        Self {
            verification_weight: 100.0,
            exit_code_zero_bonus: 50.0,
            non_empty_output_bonus: 10.0,
            latency_penalty_per_sec: 2.0,
            diff_size_penalty_per_kb: 1.0,
            min_output_bytes: 1,
        }
    }
}

/// Detailed objective scorecard produced by Auto-Judge for a single candidate engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateScorecard {
    pub candidate_id: String,
    pub engine_kind: EngineKind,
    pub passed_verifications: usize,
    pub total_verifications: usize,
    pub verification_pass_rate: f64,
    pub verification_results: Vec<VerificationResult>,
    pub exit_code: Option<i32>,
    pub execution_duration_ms: Option<u128>,
    pub diff_bytes: usize,
    pub total_score: f64,
    pub succeeded: bool,
    pub error_message: Option<String>,
}

/// Objective evaluator that scores candidate engine outputs using verification results,
/// exit code status, execution latency, and diff/output size.
#[derive(Debug, Clone, Default)]
pub struct AutoJudge {
    config: AutoJudgeConfig,
}

impl AutoJudge {
    pub fn new(config: AutoJudgeConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &AutoJudgeConfig {
        &self.config
    }

    /// Computes a deterministic scorecard for a candidate given its execution result and verifications.
    pub fn score_candidate(
        &self,
        candidate_id: impl Into<String>,
        engine_kind: EngineKind,
        output: &Result<EngineOutput, EngineError>,
        verification_results: Vec<VerificationResult>,
        diff_bytes: usize,
    ) -> CandidateScorecard {
        let id = candidate_id.into();
        let total_verifications = verification_results.len();
        let passed_verifications = verification_results.iter().filter(|r| r.passed).count();

        match output {
            Err(err) => CandidateScorecard {
                candidate_id: id,
                engine_kind,
                passed_verifications: 0,
                total_verifications,
                verification_pass_rate: 0.0,
                verification_results,
                exit_code: None,
                execution_duration_ms: None,
                diff_bytes,
                total_score: -1000.0,
                succeeded: false,
                error_message: Some(err.to_string()),
            },
            Ok(eo) => {
                let pass_rate = if total_verifications > 0 {
                    (passed_verifications as f64) / (total_verifications as f64)
                } else if eo.exit_code == 0 {
                    1.0
                } else {
                    0.0
                };

                let verification_score = pass_rate * self.config.verification_weight;
                let exit_bonus = if eo.exit_code == 0 {
                    self.config.exit_code_zero_bonus
                } else {
                    -self.config.exit_code_zero_bonus
                };
                let non_empty_bonus =
                    if eo.stdout_content.trim().len() >= self.config.min_output_bytes {
                        self.config.non_empty_output_bonus
                    } else {
                        0.0
                    };
                let latency_secs = (eo.execution_duration_ms as f64) / 1000.0;
                let latency_penalty = latency_secs * self.config.latency_penalty_per_sec;
                let diff_kb = (diff_bytes as f64) / 1024.0;
                let diff_penalty = diff_kb * self.config.diff_size_penalty_per_kb;

                let total_score = verification_score + exit_bonus + non_empty_bonus
                    - latency_penalty
                    - diff_penalty;
                let verifications_ok =
                    total_verifications == 0 || passed_verifications == total_verifications;
                let succeeded = eo.exit_code == 0 && verifications_ok;

                CandidateScorecard {
                    candidate_id: id,
                    engine_kind,
                    passed_verifications,
                    total_verifications,
                    verification_pass_rate: pass_rate,
                    verification_results,
                    exit_code: Some(eo.exit_code),
                    execution_duration_ms: Some(eo.execution_duration_ms),
                    diff_bytes,
                    total_score,
                    succeeded,
                    error_message: None,
                }
            }
        }
    }

    /// Runs `TaskVerifier` specifications against `workspace_dir` and scores the candidate.
    pub async fn evaluate_with_verifier(
        &self,
        workspace_dir: &Path,
        candidate_id: impl Into<String>,
        engine_kind: EngineKind,
        output: &Result<EngineOutput, EngineError>,
        specs: &[VerificationSpec],
        diff_bytes: usize,
    ) -> CandidateScorecard {
        let verification_results = if output.is_ok() && !specs.is_empty() {
            let verifier = TaskVerifier::new(workspace_dir);
            let (_, results) = verifier.verify_all(specs).await;
            results
        } else {
            Vec::new()
        };
        self.score_candidate(
            candidate_id,
            engine_kind,
            output,
            verification_results,
            diff_bytes,
        )
    }
}

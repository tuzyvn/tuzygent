//! # AgentCore SDK (`agent-core`)
//!
//! Enterprise Autonomous Multi-Agent Platform SDK in Rust.
//! Designed for embedding autonomous reasoning agents, OODA loops, and inter-agent
//! event collaboration directly into any Rust application.
//!
//! ## Quick Start
//! ```no_run
//! use agent_core::prelude::*;
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let bus = InterAgentBus::new(100);
//!     let supervisor = Arc::new(AgentSupervisor::new(bus.clone()));
//!
//!     let identity = AgentIdentity {
//!         id: AgentId::from_str("secops-agent"),
//!         name: "SecOps Auditor".to_string(),
//!         description: "Automated security compliance verification".to_string(),
//!         autonomy_level: AutonomyLevel::Full,
//!         created_at: chrono::Utc::now(),
//!     };
//!
//!     supervisor.register_agent(identity).await;
//!     let agents = supervisor.list_agents().await;
//!     println!("Active agents: {:?}", agents.len());
//!
//!     Ok(())
//! }
//! ```

#![deny(ambiguous_glob_reexports)]
#![deny(clippy::clone_on_copy)]
#![deny(clippy::cloned_instead_of_copied)]
#![deny(suspicious_double_ref_op)]

pub use agent_domain as domain;
pub use agent_engine as engine;
pub use agent_memory as memory;
pub use agent_runtime as runtime;

pub mod benchmark;
pub mod fault;
pub mod resilience;
pub mod safe;
pub mod sharded;

pub use benchmark::{CoreBenchmarkHarness, CoreBenchmarkStats};
pub use fault::{CircuitState, CoreExecutionError, CoreFaultKind, CoreFaultPolicy};
pub use resilience::{CoreExecutionResult, ResilientCoreAgent, ResilientCoreAgentBuilder};
pub use safe::{SafeGuard, SafeUnwrapExt};
pub use sharded::{ShardedCoreAgentRunner, SHARD_COUNT};

pub mod prelude {
    pub use agent_domain::{
        default_subagent_roster, discover_workspace_roster, escape_provenance_notes,
        format_provenance_note, format_provenance_note_with, holds_credential, is_secret_key_name,
        load_subagents_from_dir, mask_secret, redact_json_value, redact_secrets_in_text,
        roster_to_rig_spec, sanitize_note_field, wrap_with_provenance, AgentId, AgentIdentity,
        AgentMission, AgentPersonaProfile, AgentRole, AgentSoul, AutonomyLevel, GuardrailPolicy,
        InterAgentEvent, PersonaId, Priority, ProvenanceAuthority, ProvenanceEnvelope,
        RedLineRule, RigSnapshot, RigSpec, SeatSnapshot, SubTask, SubagentFrontmatter,
        SubagentSpec, Task, TaskError, TaskId, TaskStatus, VoiceEngineType, VoiceProfile,
        CONSENT_RULE, PROVENANCE_SYSTEM_RULE,
    };
    pub use agent_engine::{
        api::OpenAiCompatibleEngine, cli::AntigravityEngine, cli::ClaudeCodeEngine,
        cli::CodexEngine, cli::GenericCliEngine, config::EnginesConfig, config::SingleEngineConfig,
        router::EngineRouter, ComputeEngine, EngineInput, EngineKind, EngineOutput,
    };
    pub use agent_memory::{
        DurableMemoryStore, MemoryRecord, MemoryService, PersonaLoader, RetrievedContext,
        ScopeAccess, ScopedScratchpad, ScratchpadManager, SkillRegistry,
    };
    pub use agent_runtime::{
        classify_shell_error, failure_signature, filter_sandbox_env_whitelist,
        is_sandbox_env_allowed, normalize_signature_text, verify_sandbox_plan, AgentSupervisor,
        AutoJudge, AutoJudgeConfig, AutonomousOodaLoop, CandidateScorecard, ExecutionLoopConfig,
        ExecutionLoopGuard, FailureSignature, InterAgentBus, IsolationMode, KernelOperator,
        LoopVerdict, RepeatDetector, RepeatObservation, ResilientWorkspaceManager, RoomTargetInfo,
        SandboxConfig, SandboxDriverKind, SandboxEnvironment, SandboxMutation,
        SandboxSelfCheckReport, SandboxSpec, TeammateTurnResult, TournamentCandidate,
        TournamentExecutor, TournamentOutcome, WorkflowOrchestrator, WorkspaceBenchmarkHarness,
        WorkspaceManager, WsRpcClient, CoordinateBotsRequest, CoordinationBatchResult,
        CoordinationContext, CoordinationReplyMode, DEFAULT_MAX_COORDINATION_DEPTH,
        MAX_COORDINATE_BOTS, SANDBOX_CPU_TOPOLOGY_PATHS, SANDBOX_ENV_NEVER_LIST,
    };

    pub use crate::benchmark::{CoreBenchmarkHarness, CoreBenchmarkStats};
    pub use crate::fault::{CircuitState, CoreExecutionError, CoreFaultKind, CoreFaultPolicy};
    pub use crate::resilience::{
        CoreExecutionResult, ResilientCoreAgent, ResilientCoreAgentBuilder,
    };
    pub use crate::safe::{SafeGuard, SafeUnwrapExt};
    pub use crate::sharded::{ShardedCoreAgentRunner, SHARD_COUNT};
}

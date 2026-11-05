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

pub use agent_domain as domain;
pub use agent_engine as engine;
pub use agent_memory as memory;
pub use agent_runtime as runtime;

pub mod prelude {
    pub use agent_domain::{
        PersonaId, AgentId, AgentIdentity, AgentMission, AgentPersonaProfile, AgentRole, AgentSoul,
        AutonomyLevel, GuardrailPolicy, InterAgentEvent, Priority, RedLineRule, RigSnapshot,
        RigSpec, SeatSnapshot, SubTask, Task, TaskId, TaskStatus, VoiceEngineType, VoiceProfile,
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
        AgentSupervisor, AutonomousOodaLoop, InterAgentBus, IsolationMode, KernelOperator,
        SandboxConfig, SandboxDriverKind, SandboxEnvironment, SandboxSpec, WorkflowOrchestrator, WsRpcClient,
    };
}

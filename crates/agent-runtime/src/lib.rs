#![deny(ambiguous_glob_reexports)]

pub mod bus;
pub mod execution;
pub mod harness;
pub mod ooda;
pub use ooda as ooda_loop;
pub mod operator;
pub mod processtree;
pub mod report_queue;
pub mod sandbox;
pub mod supervisor;
pub mod supplement;
pub mod task_service;
pub mod task_store;
pub mod verifier;
pub mod watchdog;
pub mod workflow_dag;
pub mod wsrpc;

// Explicit re-exports: eliminates all ambiguous_glob_reexports risks
pub use agent_memory::{ScopeAccess, ScopedScratchpad};
pub use bus::{
    simulate_bus_fault, BusBenchmarkHarness, BusBenchmarkStats, BusError, BusEventFilter, BusFault,
    BusFaultPolicy, InterAgentBus, ResilientInterAgentBus, ResilientSubscriber, ShardedBusQueue,
};
pub use execution::{
    is_retryable_execution_error, simulate_execution_fault, ExecutionBenchmarkHarness,
    ExecutionBenchmarkStats, ExecutionFault, ExecutionFaultPolicy, ExecutionService,
    ResilientExecutionService,
};
pub use harness::{
    is_retryable_harness_error, simulate_harness_fault, HarnessBenchmarkHarness,
    HarnessBenchmarkStats, HarnessError, HarnessFault, HarnessFaultPolicy, HarnessLesson,
    HarnessLessonCache, HarnessMemoryStore, HarnessSelfHealer, ResilientHarnessMemory,
    ResilientHarnessMemoryStore,
};
pub use ooda::{
    AutonomousOodaLoop, GoalDecomposer, QueueExecutionOutcome, SubtaskExecutor,
    MAX_SUBTASK_RETRIES, MAX_TOTAL_REMEDIATIONS,
    OODA_PHASE_ACT, OODA_PHASE_DECIDE, OODA_PHASE_OBSERVE, OODA_PHASE_ORIENT, OODA_PHASE_REMEDIATE,
};
pub use operator::{KernelOperator, RigExecutionSummary, RigSeatAssignment, SeatStatusReport};
pub use processtree::ProcessTreeController;
pub use report_queue::{TerminalReportKind, TerminalReportStore, TerminalTaskReport};
pub use sandbox::{
    expand_home, inject_agent_brief, purge_sidecar_context, IsolationMode, SandboxConfig,
    SandboxDriverKind, SandboxEnvironment, SandboxSpec, SidecarContextManager, WorkspaceManager,
};
pub use supervisor::{AgentState, AgentSupervisor};
pub use supplement::{InFlightSupplement, InFlightSupplementController};
pub use task_service::TaskService;
pub use task_store::{IdempotentSubmission, SqliteTaskStore, TaskStore};
pub use verifier::TaskVerifier;
pub use watchdog::{IdleWatchdog, MonotonicClock};
pub use workflow_dag::{
    execute_agent_node, execute_approval_node, execute_consensus_gate_node, execute_node,
    execute_router_node, execute_sub_workflow_node, execute_tool_node, execute_trigger_node,
    validate_workflow, NodeExecutionContext, NodeExecutionOutcome, WorkflowDag, WorkflowDefinition,
    WorkflowEdge, WorkflowNode, WorkflowNodeData, WorkflowNodeExecutionResult,
    WorkflowOrchestrator, WorkflowRunReport, WorkflowTimelineEvent,
};
pub use wsrpc::{WsOutboundFrame, WsRpcClient, WsRpcRequest, WsRpcResponse};

#![deny(ambiguous_glob_reexports)]
#![deny(clippy::clone_on_copy)]
#![deny(clippy::cloned_instead_of_copied)]
#![deny(suspicious_double_ref_op)]

pub mod bus;
pub mod execution;
pub mod harness;
pub mod mcp_gateway;
pub mod ooda;
pub use ooda as ooda_loop;
pub mod operator;
pub mod processtree;
pub mod report_queue;
pub mod sandbox;
pub mod sentinel;
pub mod supervisor;
pub mod supplement;
pub mod task_service;
pub mod task_store;
pub mod tournament;
pub mod verifier;
pub mod watchdog;
pub mod workflow_dag;
pub mod wsrpc;

// Explicit re-exports: eliminates all ambiguous_glob_reexports risks
pub use agent_engine::BackoffPolicy;
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
pub use mcp_gateway::{
    FnMcpToolHandler, McpContentBlock, McpGatewayError, McpJsonRpcError, McpJsonRpcRequest,
    McpJsonRpcResponse, McpPropertyType, McpServerConfig, McpToolCallParams, McpToolCallResult,
    McpToolDefinition, McpToolGateway, McpToolHandler, McpToolSchema, JSONRPC_VERSION,
    MCP_ERR_EXECUTION_FAILED, MCP_ERR_INVALID_PARAMS, MCP_ERR_INVALID_REQUEST,
    MCP_ERR_METHOD_NOT_FOUND, MCP_ERR_PARSE_ERROR, MCP_ERR_POLICY_BLOCKED,
    MCP_ERR_SERVER_UNAVAILABLE,
};
pub use ooda::{
    AutonomousOodaLoop, GoalDecomposer, QueueExecutionOutcome, SubtaskExecutor,
    MAX_SUBTASK_RETRIES, MAX_TOTAL_REMEDIATIONS, OODA_PHASE_ACT, OODA_PHASE_DECIDE,
    OODA_PHASE_OBSERVE, OODA_PHASE_ORIENT, OODA_PHASE_REMEDIATE,
};
pub use operator::{
    CoordinateBotsRequest, CoordinationBatchResult, CoordinationContext, CoordinationReplyMode,
    KernelOperator, RigExecutionSummary, RigSeatAssignment, RoomTargetInfo, SeatStatusReport,
    TeammateTurnResult, DEFAULT_MAX_COORDINATION_DEPTH, MAX_COORDINATE_BOTS,
};
pub use processtree::ProcessTreeController;
pub use report_queue::{TerminalReportKind, TerminalReportStore, TerminalTaskReport};
pub use sandbox::{
    command_sandbox, expand_home, filter_sandbox_env_whitelist, inject_agent_brief,
    is_retryable_workspace_error, is_sandbox_env_allowed, prepare_workspace, purge_sidecar_context,
    teardown_workspace, verify_sandbox_plan, wrap_command, CommandManager, IsolationMode,
    ResilientWorkspaceManager, SandboxConfig, SandboxDriverKind, SandboxEnvironment,
    SandboxMutation, SandboxSelfCheckReport, SandboxSpec, SidecarContextManager,
    WorkspaceAuditRecord, WorkspaceBenchmarkHarness, WorkspaceBenchmarkStats, WorkspaceError,
    WorkspaceFaultKind, WorkspaceFaultPolicy, WorkspaceManager, SANDBOX_CPU_TOPOLOGY_PATHS,
    SANDBOX_ENV_NEVER_LIST,
};
pub use sentinel::{
    classify_shell_error, failure_signature, normalize_signature_text, ExecutionLoopConfig,
    ExecutionLoopGuard, FailureSignature, LoopVerdict, RepeatDetector, RepeatObservation,
};
pub use supervisor::{AgentState, AgentSupervisor};
pub use supplement::{InFlightSupplement, InFlightSupplementController};
pub use task_service::{SchedulerTickReport, TaskService};
pub use task_store::{IdempotentSubmission, SqliteTaskStore, TaskStore};
pub use tournament::{
    AutoJudge, AutoJudgeConfig, CandidateScorecard, TournamentCandidate, TournamentExecutor,
    TournamentOutcome,
};
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

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
pub use bus::InterAgentBus;
pub use execution::ExecutionService;
pub use harness::{HarnessLesson, HarnessMemoryStore, HarnessSelfHealer};
pub use ooda::{
    AutonomousOodaLoop, GoalDecomposer, QueueExecutionOutcome, SubtaskExecutor,
    MAX_SUBTASK_RETRIES, MAX_TOTAL_REMEDIATIONS,
};
pub use operator::{KernelOperator, RigExecutionSummary, RigSeatAssignment, SeatStatusReport};
pub use processtree::ProcessTreeController;
pub use report_queue::{TerminalReportKind, TerminalReportStore, TerminalTaskReport};
pub use sandbox::{
    expand_home, inject_agent_brief, purge_sidecar_context, IsolationMode, SandboxConfig,
    SandboxDriverKind, SandboxEnvironment, SidecarContextManager, WorkspaceManager,
};
pub use supervisor::{AgentState, AgentSupervisor};
pub use supplement::{InFlightSupplement, InFlightSupplementController};
pub use task_service::TaskService;
pub use task_store::{SqliteTaskStore, TaskStore};
pub use verifier::TaskVerifier;
pub use watchdog::{IdleWatchdog, MonotonicClock};
pub use workflow_dag::{
    execute_agent_node, execute_approval_node, execute_node, execute_router_node,
    execute_sub_workflow_node, execute_tool_node, execute_trigger_node, validate_workflow,
    NodeExecutionContext, NodeExecutionOutcome, WorkflowDefinition, WorkflowEdge, WorkflowNode,
    WorkflowNodeData, WorkflowNodeExecutionResult, WorkflowOrchestrator, WorkflowRunReport,
    WorkflowTimelineEvent,
};
pub use wsrpc::{WsOutboundFrame, WsRpcClient, WsRpcRequest, WsRpcResponse};

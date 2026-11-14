#![deny(ambiguous_glob_reexports)]
#![deny(clippy::clone_on_copy)]
#![deny(clippy::cloned_instead_of_copied)]
#![deny(suspicious_double_ref_op)]

pub mod audit;
pub mod audit_ledger;
pub mod checkpoint;
pub mod event;
pub mod guardrail;
pub mod identity;
pub mod provenance;
pub mod redact;
pub mod rig;
pub mod schedule;
pub mod subagent;
pub mod task;

pub use audit_ledger::*;
pub use checkpoint::*;
pub use event::*;
pub use guardrail::*;
pub use identity::*;
pub use provenance::*;
pub use redact::*;
pub use rig::*;
pub use schedule::*;
pub use subagent::{
    default_subagent_roster, discover_workspace_roster, load_subagents_from_dir,
    roster_to_rig_spec, SubagentFrontmatter, SubagentSpec,
};
pub use task::*;

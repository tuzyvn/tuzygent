//! Project-scoped persona memory. Callers supply trusted project/persona context;
//! model output must never select that context or assert verification success.

pub mod legacy;
pub mod lifecycle;
pub mod profile;
pub mod retrieval;
pub mod schema;
pub mod service;
pub mod types;

pub use service::MemoryService;
pub use types::{MemoryRecord, RetrievedContext};

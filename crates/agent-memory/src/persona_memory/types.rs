use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub id: String,
    pub persona_id: String,
    pub kind: String,
    pub scope: String,
    pub status: String,
    pub content: String,
    pub conditions: String,
    pub source: String,
    pub verified: bool,
    pub profile_version: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievedContext {
    pub memory_ids: Vec<String>,
    pub text: String,
}

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsRpcRequest {
    pub request_id: String,
    pub method: String,
    pub payload: serde_json::Value,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsRpcResponse {
    pub request_id: String,
    pub status: u16,
    pub payload: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WsOutboundFrame {
    pub data: Vec<u8>,
    pub sent: bool,
    pub canceled: bool,
}

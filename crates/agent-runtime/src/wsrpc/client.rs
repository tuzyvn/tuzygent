use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::{debug, info, warn};

use super::types::{WsRpcRequest, WsRpcResponse};

/// Daemon WS-First Multiplexed RPC Client theo chuẩn Multica (MUL-4257)
pub struct WsRpcClient {
    generation: AtomicU64,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<WsRpcResponse>>>>,
    outbound_tx: Arc<Mutex<Option<mpsc::Sender<WsRpcRequest>>>>,
    grace_duration: Duration,
}

impl WsRpcClient {
    pub fn new(grace_duration: Duration) -> Self {
        Self {
            generation: AtomicU64::new(0),
            pending: Arc::new(Mutex::new(HashMap::new())),
            outbound_tx: Arc::new(Mutex::new(None)),
            grace_duration,
        }
    }

    /// Lấy thế hệ (generation) kết nối hiện tại
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Kiểm tra xem client có đang kết nối với kênh WebSocket outbound hay không
    pub async fn is_connected(&self) -> bool {
        self.outbound_tx.lock().await.is_some()
    }

    /// Đính kèm kênh WebSocket active
    pub async fn attach(&self, tx: mpsc::Sender<WsRpcRequest>) -> u64 {
        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let mut out = self.outbound_tx.lock().await;
        *out = Some(tx);

        // Huỷ bỏ và làm sạch các request đang treo của connection trước đó
        let mut pending = self.pending.lock().await;
        pending.clear();

        info!(
            "🔌 [WsRpcClient] Attached new connection (generation={})",
            gen
        );
        gen
    }

    /// Tháo gỡ khi kết nối bị rớt
    pub async fn detach(&self) {
        let mut out = self.outbound_tx.lock().await;
        *out = None;
        let mut pending = self.pending.lock().await;
        pending.clear();
        warn!("🔌 [WsRpcClient] Detached connection");
    }

    /// Gọi RPC bất đồng bộ với khả năng failover và chống double-claim
    pub async fn call(
        &self,
        method: &str,
        payload: serde_json::Value,
        timeout: Duration,
    ) -> Result<WsRpcResponse, String> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let (resp_tx, resp_rx) = oneshot::channel();

        let req = WsRpcRequest {
            request_id: request_id.clone(),
            method: method.to_string(),
            payload,
            timeout_ms: timeout.as_millis() as u64,
        };

        // 1. Kiểm tra kết nối và đăng ký pending request
        {
            // Match attach/detach lock order: outbound, then pending.
            let out_guard = self.outbound_tx.lock().await;
            let mut pending = self.pending.lock().await;
            if let Some(ref tx) = *out_guard {
                if let Err(e) = tx.try_send(req) {
                    return match e {
                        mpsc::error::TrySendError::Full(_) => {
                            Err(format!("WS outbound buffer saturated: {}", e))
                        }
                        mpsc::error::TrySendError::Closed(_) => {
                            Err(format!("WS outbound channel closed: {}", e))
                        }
                    };
                }
                pending.insert(request_id.clone(), resp_tx);
            } else {
                return Err("WS RPC unavailable: no active connection".to_string());
            }
        }

        // 2. Chờ phản hồi với ngân sách Timeout + Grace margin (Multica pattern)
        let total_timeout = timeout + self.grace_duration;
        match tokio::time::timeout(total_timeout, resp_rx).await {
            Ok(Ok(response)) => {
                if response.status >= 200 && response.status < 300 {
                    Ok(response)
                } else {
                    Err(response
                        .error
                        .unwrap_or_else(|| format!("RPC status {}", response.status)))
                }
            }
            Ok(Err(_)) => {
                // Kênh bị đóng do rớt mạng giữa chừng (Uncertain outcome)
                let mut pending = self.pending.lock().await;
                pending.remove(&request_id);
                Err("WS RPC uncertain: sent but connection dropped before response".to_string())
            }
            Err(_) => {
                // Quá hạn timeout
                let mut pending = self.pending.lock().await;
                pending.remove(&request_id);
                Err(format!("WS RPC timed out after {:?}", total_timeout))
            }
        }
    }

    /// Nhận khung tin phản hồi từ Server qua luồng đọc WS
    pub async fn deliver_response(&self, response: WsRpcResponse) {
        let mut pending = self.pending.lock().await;
        if let Some(tx) = pending.remove(&response.request_id) {
            let _ = tx.send(response);
        } else {
            debug!(
                "Discarded WS RPC response for unknown/expired request_id: {}",
                response.request_id
            );
        }
    }
}

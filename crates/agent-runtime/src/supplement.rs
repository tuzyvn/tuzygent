use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{info, warn};

/// Đại diện cho một chỉ thị bổ sung khi Agent đang thực thi
#[derive(Debug, Clone)]
pub struct InFlightSupplement {
    pub author_name: String,
    pub content: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl InFlightSupplement {
    pub fn new(author_name: &str, content: &str) -> Self {
        Self {
            author_name: author_name.to_string(),
            content: content.to_string(),
            timestamp: chrono::Utc::now(),
        }
    }

    /// Định dạng chỉ thị bổ sung theo chuẩn Multica prompt engineering:
    /// Giữ nguyên mục tiêu ban đầu, lồng ghép chỉ thị mới vào cùng lượt chạy.
    pub fn to_instruction_block(&self) -> String {
        let author = if self.author_name.trim().is_empty() {
            "a user".to_string()
        } else {
            self.author_name.clone()
        };

        format!(
            "[ADDITIONAL GUIDANCE] Human \"{}\" added guidance while you were working.\n\n\
            Treat this as additional guidance for the same active task, not as a replacement:\n\
            - Preserve and complete the original objective.\n\
            - Incorporate this guidance into the work and the turn's single final response.\n\
            - Do not send a separate acknowledgement.\n\
            - Replace or cancel the original objective only if the human explicitly asks for replacement or cancellation.\n\n\
            Human message:\n{}",
            author, self.content
        )
    }
}

/// Kênh điều phối chỉ thị thời gian thực cho một Agent đang chạy
#[derive(Clone)]
pub struct InFlightSupplementController {
    tx: mpsc::Sender<InFlightSupplement>,
    rx: Arc<RwLock<mpsc::Receiver<InFlightSupplement>>>,
}

impl InFlightSupplementController {
    pub fn new(capacity: usize) -> Self {
        let (tx, rx) = mpsc::channel(capacity);
        Self {
            tx,
            rx: Arc::new(RwLock::new(rx)),
        }
    }

    /// Người dùng hoặc hệ thống gửi chỉ thị can thiệp vào giữa chừng
    pub async fn inject(&self, supplement: InFlightSupplement) -> Result<(), String> {
        info!(
            "📢 [Supplement Controller] Injected guidance from '{}'",
            supplement.author_name
        );
        self.tx
            .send(supplement)
            .await
            .map_err(|e| format!("Failed to inject supplement into running turn: {}", e))
    }

    /// Lấy chỉ thị bổ sung mới nhất nếu có mà không chặn (non-blocking)
    pub async fn try_recv(&self) -> Option<InFlightSupplement> {
        let mut rx = self.rx.write().await;
        match rx.try_recv() {
            Ok(sup) => Some(sup),
            Err(mpsc::error::TryRecvError::Empty) => None,
            Err(mpsc::error::TryRecvError::Disconnected) => {
                warn!("⚠️ [Supplement Controller] Channel disconnected");
                None
            }
        }
    }
}

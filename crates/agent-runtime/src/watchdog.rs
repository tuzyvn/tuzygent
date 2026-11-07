use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::watch;
use tokio::time::sleep;
use tracing::{info, warn};

/// Đồng hồ đo thời gian hoạt động (Awake Clock)
/// Dựa trên Instant của Rust monotonic clock, không bị trôi do đồng hồ hệ thống thay đổi
#[derive(Clone)]
pub struct MonotonicClock {
    start: Instant,
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}

impl MonotonicClock {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }

    pub fn elapsed_millis(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }
}

/// Giám sát liveness và tự động thu hồi/ngắt tiến trình Agent bị treo (Hang / Freeze)
/// Triển khai theo mẫu Multica `runIdleWatchdog`
pub struct IdleWatchdog {
    window: Duration,
    tool_window: Duration,
    last_activity_millis: Arc<AtomicU64>,
    in_flight_tools: Arc<AtomicU64>,
    fired: Arc<AtomicBool>,
    shutdown_tx: watch::Sender<bool>,
    clock: MonotonicClock,
}

impl IdleWatchdog {
    pub fn new(window: Duration, tool_window: Duration) -> (Self, watch::Receiver<bool>) {
        let (tx, rx) = watch::channel(false);
        let clock = MonotonicClock::new();
        let initial_millis = clock.elapsed_millis();

        let watchdog = Self {
            window,
            tool_window,
            last_activity_millis: Arc::new(AtomicU64::new(initial_millis)),
            in_flight_tools: Arc::new(AtomicU64::new(0)),
            fired: Arc::new(AtomicBool::new(false)),
            shutdown_tx: tx,
            clock,
        };

        (watchdog, rx)
    }

    /// Đánh dấu Agent vừa phát ra tín hiệu (text, tool call, reasoning token)
    pub fn mark_activity(&self) {
        let now = self.clock.elapsed_millis();
        self.last_activity_millis.store(now, Ordering::Release);
    }

    /// Ghi nhận công cụ (tool) bắt đầu chạy
    pub fn enter_tool(&self) {
        self.in_flight_tools.fetch_add(1, Ordering::SeqCst);
        self.mark_activity();
    }

    /// Ghi nhận công cụ (tool) kết thúc
    pub fn exit_tool(&self) {
        let _ = self
            .in_flight_tools
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |val| {
                Some(val.saturating_sub(1))
            });
        self.mark_activity();
    }

    pub fn is_fired(&self) -> bool {
        self.fired.load(Ordering::Acquire)
    }

    /// Vòng lặp giám sát định kỳ bất đồng bộ
    pub async fn start_monitoring(self: Arc<Self>, agent_id: String) {
        let tick_duration = Duration::from_millis(500); // 500ms heartbeat
        info!("⏱️ [IdleWatchdog] Started liveness watchdog for agent '{}' (idle_budget={:?}, tool_budget={:?})",
            agent_id, self.window, self.tool_window);

        loop {
            sleep(tick_duration).await;

            if self.is_fired() {
                break;
            }

            let in_flight = self.in_flight_tools.load(Ordering::Relaxed);
            let threshold = if in_flight > 0 {
                self.tool_window
            } else {
                self.window
            };

            let now = self.clock.elapsed_millis();
            let last = self.last_activity_millis.load(Ordering::Acquire);
            let idle_time = Duration::from_millis(now.saturating_sub(last));

            if idle_time >= threshold {
                warn!("🚨 [IdleWatchdog] AGENT HANG DETECTED! Agent '{}' silent for {:?} (threshold={:?}, in_flight_tools={}). Tripping watchdog!",
                    agent_id, idle_time, threshold, in_flight);
                self.fired.store(true, Ordering::Release);
                let _ = self.shutdown_tx.send(true);
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{IdleWatchdog, MonotonicClock};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::time::{sleep, timeout};

    #[test]
    fn monotonic_clock_starts_at_zero_and_advances() {
        let clock = MonotonicClock::new();
        assert_eq!(clock.elapsed_millis(), 0);

        std::thread::sleep(Duration::from_millis(5));
        assert!(clock.elapsed_millis() >= 5);
    }

    #[tokio::test]
    async fn new_watchdog_starts_unfired_and_monitor_reports_idle_timeout() {
        let (watchdog, mut shutdown_rx) = IdleWatchdog::new(Duration::ZERO, Duration::from_secs(1));
        assert!(!watchdog.is_fired());
        assert!(!*shutdown_rx.borrow());

        Arc::new(watchdog)
            .start_monitoring("idle-agent".to_owned())
            .await;

        assert!(shutdown_rx.changed().await.is_ok());
        assert!(*shutdown_rx.borrow());
    }

    #[tokio::test]
    async fn mark_activity_resets_the_idle_timeout() {
        let (watchdog, mut shutdown_rx) =
            IdleWatchdog::new(Duration::from_millis(700), Duration::from_secs(1));
        let watchdog = Arc::new(watchdog);
        let monitor = tokio::spawn(Arc::clone(&watchdog).start_monitoring("active-agent".into()));

        sleep(Duration::from_millis(550)).await;
        watchdog.mark_activity();
        sleep(Duration::from_millis(150)).await;
        assert!(!watchdog.is_fired());
        assert!(!*shutdown_rx.borrow());

        timeout(Duration::from_secs(2), shutdown_rx.changed())
            .await
            .expect("watchdog should eventually detect inactivity")
            .expect("shutdown receiver should remain connected");
        assert!(watchdog.is_fired());
        monitor.await.expect("monitor task should finish");
    }

    #[tokio::test]
    async fn tool_activity_uses_tool_window_until_tool_exits() {
        let (watchdog, mut shutdown_rx) =
            IdleWatchdog::new(Duration::from_millis(10), Duration::from_secs(2));
        watchdog.enter_tool();
        let watchdog = Arc::new(watchdog);
        let monitor = tokio::spawn(Arc::clone(&watchdog).start_monitoring("tool-agent".into()));

        // The idle window has elapsed, but the active tool still has its longer budget.
        sleep(Duration::from_millis(600)).await;
        assert!(!watchdog.is_fired());
        assert!(!*shutdown_rx.borrow());

        watchdog.exit_tool();
        timeout(Duration::from_secs(1), shutdown_rx.changed())
            .await
            .expect("watchdog should use the idle window after the tool exits")
            .expect("shutdown receiver should remain connected");
        assert!(watchdog.is_fired());
        monitor.await.expect("monitor task should finish");
    }
}

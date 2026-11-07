use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use super::*;

#[tokio::test]
async fn reconnect_and_calls_do_not_deadlock() -> Result<(), Box<dyn std::error::Error>> {
    let client = Arc::new(WsRpcClient::new(Duration::ZERO));
    let reconnect = client.clone();
    let reconnect_task = tokio::spawn(async move {
        for _ in 0..100 {
            let (tx, mut rx) = mpsc::channel(8);
            reconnect.attach(tx).await;
            let responder = reconnect.clone();
            tokio::spawn(async move {
                while let Some(request) = rx.recv().await {
                    responder
                        .deliver_response(WsRpcResponse {
                            request_id: request.request_id,
                            status: 200,
                            payload: None,
                            error: None,
                        })
                        .await;
                }
            });
            tokio::task::yield_now().await;
            reconnect.detach().await;
        }
    });
    let calls = tokio::spawn(async move {
        for _ in 0..100 {
            let _ = client
                .call("probe", serde_json::json!({}), Duration::from_millis(10))
                .await;
            tokio::task::yield_now().await;
        }
    });
    let (reconnect_res, calls_res) = tokio::time::timeout(Duration::from_secs(5), async {
        let res_reconnect = reconnect_task.await;
        let res_calls = calls.await;
        (res_reconnect, res_calls)
    })
    .await
    .map_err(|_| "Test timed out due to lock order deadlock")?;

    reconnect_res?;
    calls_res?;
    Ok(())
}

#[tokio::test]
async fn response_is_delivered_to_pending_call() -> Result<(), Box<dyn std::error::Error>> {
    let client = Arc::new(WsRpcClient::new(Duration::ZERO));
    let (tx, mut rx) = mpsc::channel(1);
    client.attach(tx).await;
    let caller = client.clone();
    let task = tokio::spawn(async move {
        caller
            .call("probe", serde_json::json!({}), Duration::from_secs(1))
            .await
    });
    let request = rx
        .recv()
        .await
        .ok_or("Channel closed unexpectedly before receiving request")?;
    client
        .deliver_response(WsRpcResponse {
            request_id: request.request_id,
            status: 200,
            payload: Some(serde_json::json!("ok")),
            error: None,
        })
        .await;
    let call_res = task.await?;
    let response = call_res.map_err(|e| format!("WsRpcClient::call failed: {}", e))?;
    assert_eq!(
        response.payload,
        Some(serde_json::json!("ok")),
        "Response payload must match expected value"
    );
    Ok(())
}

#[tokio::test]
async fn channel_closed_returns_error_without_panic() -> Result<(), Box<dyn std::error::Error>> {
    let client = Arc::new(WsRpcClient::new(Duration::from_millis(50)));
    let (tx, rx) = mpsc::channel(1);
    client.attach(tx).await;

    // Đóng receiver ngay lập tức để mô phỏng channel bị đóng
    drop(rx);

    // Gọi RPC khi channel đã đóng: try_send sẽ trả về Err Closed an toàn, không gây panic
    let res = client
        .call("probe", serde_json::json!({}), Duration::from_millis(50))
        .await;

    let err = match res {
        Err(e) => e,
        Ok(v) => return Err(format!("Expected error when channel closed, got Ok({:?})", v).into()),
    };
    assert!(
        err.contains("WS outbound channel closed"),
        "Error should describe closed channel: {}",
        err
    );
    Ok(())
}

#[tokio::test]
async fn buffer_saturated_returns_error_without_panic() -> Result<(), Box<dyn std::error::Error>> {
    let client = Arc::new(WsRpcClient::new(Duration::from_millis(50)));
    // Buffer capacity = 1
    let (tx, _rx) = mpsc::channel(1);
    client.attach(tx.clone()).await;

    // Lấp đầy capacity 1 bằng tx
    let dummy_req = WsRpcRequest {
        request_id: "preload-slot".to_string(),
        method: "test".to_string(),
        payload: serde_json::json!({}),
        timeout_ms: 50,
    };
    tx.try_send(dummy_req)
        .map_err(|e| format!("Preloading buffer failed: {}", e))?;

    // Cuộc gọi tiếp theo khi buffer đã đầy (1/1) sẽ kích hoạt TrySendError::Full
    let res = client
        .call("second", serde_json::json!({}), Duration::from_millis(10))
        .await;

    let err = match res {
        Err(e) => e,
        Ok(v) => return Err(format!("Expected buffer saturated error, got Ok({:?})", v).into()),
    };
    assert!(
        err.contains("WS outbound buffer saturated"),
        "Error should describe saturated buffer: {}",
        err
    );
    Ok(())
}

#[tokio::test]
async fn connection_state_and_generation_lifecycle() -> Result<(), Box<dyn std::error::Error>> {
    let client = Arc::new(WsRpcClient::new(Duration::ZERO));
    assert_eq!(client.generation(), 0);
    assert!(!client.is_connected().await);

    let (tx, _rx) = mpsc::channel(1);
    let gen = client.attach(tx).await;
    assert_eq!(gen, 1);
    assert_eq!(client.generation(), 1);
    assert!(client.is_connected().await);

    client.detach().await;
    assert_eq!(client.generation(), 1);
    assert!(!client.is_connected().await);
    Ok(())
}

#[tokio::test]
async fn zero_timeout_handled_gracefully_without_panic() -> Result<(), Box<dyn std::error::Error>> {
    let client = Arc::new(WsRpcClient::new(Duration::ZERO));
    let (tx, _rx) = mpsc::channel(2);
    client.attach(tx).await;

    let res = client
        .call("probe", serde_json::json!({}), Duration::ZERO)
        .await;
    let err = match res {
        Err(e) => e,
        Ok(v) => return Err(format!("Expected timeout error, got Ok({:?})", v).into()),
    };
    assert!(
        err.contains("WS RPC timed out"),
        "Error should describe timeout: {}",
        err
    );
    Ok(())
}

#[tokio::test]
async fn connection_drop_during_call_returns_uncertain_without_panic(
) -> Result<(), Box<dyn std::error::Error>> {
    let client = Arc::new(WsRpcClient::new(Duration::from_millis(500)));
    let (tx, mut rx) = mpsc::channel(1);
    client.attach(tx).await;

    let caller = client.clone();
    let call_handle = tokio::spawn(async move {
        caller
            .call("probe", serde_json::json!({}), Duration::from_millis(500))
            .await
    });

    // Nhận request từ channel nhưng không deliver response, thay vào đó detach connection
    let _request = rx.recv().await.ok_or("Expected request to be received")?;

    // Mô phỏng rớt mạng / detach khiến pending map bị clear và oneshot sender bị drop
    client.detach().await;

    // Xử lý join handle an toàn mà không gọi unwrap() mù quáng
    let call_result = call_handle.await?;

    let err = match call_result {
        Err(e) => e,
        Ok(v) => {
            return Err(format!("Expected uncertain connection drop error, got Ok({:?})", v).into())
        }
    };
    assert!(
        err.contains("WS RPC uncertain"),
        "Expected uncertain connection drop error, got: {}",
        err
    );
    Ok(())
}

#[tokio::test]
async fn task_join_failure_handled_without_panic() -> Result<(), Box<dyn std::error::Error>> {
    let client = Arc::new(WsRpcClient::new(Duration::from_millis(100)));

    // 1. Kiểm tra JoinError do task bị aborted/cancelled
    let caller = client.clone();
    let aborted_task = tokio::spawn(async move {
        caller
            .call("probe", serde_json::json!({}), Duration::from_secs(1))
            .await
    });

    aborted_task.abort();
    let aborted_res = aborted_task.await;
    let join_err = match aborted_res {
        Err(e) => e,
        Ok(v) => return Err(format!("Expected task to be aborted, got Ok({:?})", v).into()),
    };
    assert!(
        join_err.is_cancelled(),
        "JoinError must indicate cancellation"
    );

    // 2. Kiểm tra JoinError do task bị panic nội bộ
    let panicking_task = tokio::spawn(async move {
        std::panic::resume_unwind(Box::new("Simulated internal task panic"));
        #[allow(unreachable_code)]
        Result::<WsRpcResponse, String>::Err("unreachable".to_string())
    });

    let panicking_res = panicking_task.await;
    let join_err = match panicking_res {
        Err(e) => e,
        Ok(v) => return Err(format!("Expected task to panic, got Ok({:?})", v).into()),
    };
    assert!(
        join_err.is_panic(),
        "JoinError must indicate panic without crashing the caller"
    );
    Ok(())
}

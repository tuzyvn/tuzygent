use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::sleep;

use agent_domain::AuditLedger;
use agent_engine::{
    simulate_api_fault, ApiBenchmarkHarness, ApiBenchmarkStats, ApiError, ApiFault,
    ApiFaultPolicy, BenchmarkMockApiEngine, ComputeEngine, CustomAgentApiEngine, EngineError,
    EngineInput, EngineKind, EngineOutput, OpenAiCompatibleEngine, ResilientApiEngine,
};

fn create_test_input(prompt: &str, timeout_seconds: u64) -> EngineInput {
    EngineInput {
        on_event: None,
        command_sandbox: None,
        prompt_or_command: prompt.to_string(),
        system_context: "You are a test agent".to_string(),
        working_dir: ".".to_string(),
        timeout_seconds,
        on_stream_chunk: None,
        temperature: None,
        max_tokens: None,
        max_output_bytes: None,
    }
}

/// Helper to read incoming HTTP headers up to "\r\n\r\n"
async fn read_http_request(stream: &mut TcpStream) -> Result<String, std::io::Error> {
    let mut buffer = [0u8; 4096];
    let mut total_read = Vec::new();

    loop {
        let n = stream.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        total_read.extend_from_slice(&buffer[..n]);
        if total_read.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&total_read).to_string())
}

#[tokio::test]
async fn test_custom_api_baseline_success() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{}", addr);

    let handle = tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let _req = read_http_request(&mut stream).await;
            let resp = "HTTP/1.1 200 OK\r\nContent-Length: 15\r\n\r\n{\"status\":\"ok\"}";
            let _ = stream.write_all(resp.as_bytes()).await;
        }
    });

    let engine = CustomAgentApiEngine::new(&url, None);
    let input = create_test_input("ping", 5);
    let output = engine.execute(&input).await.expect("execute should succeed");

    assert_eq!(output.exit_code, 0);
    assert!(output.stdout_content.contains("{\"status\":\"ok\"}"));
    handle.await.unwrap();
}

#[tokio::test]
async fn test_openai_compat_baseline_success() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    let handle = tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let _req = read_http_request(&mut stream).await;
            let json_body = serde_json::json!({
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": "Hello from mock LLM"
                    }
                }]
            })
            .to_string();
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                json_body.len(),
                json_body
            );
            let _ = stream.write_all(resp.as_bytes()).await;
        }
    });

    let engine = OpenAiCompatibleEngine::new(&base_url, "test-key", "test-model");
    let input = create_test_input("hello", 5);
    let output = engine.execute(&input).await.expect("execute should succeed");

    assert_eq!(output.exit_code, 0);
    assert_eq!(output.stdout_content, "Hello from mock LLM");
    handle.await.unwrap();
}

#[tokio::test]
async fn test_custom_api_intermittent_socket_timeout_unrecovered() {
    // Mô phỏng server bị trễ (socket timeout) ở lần gọi đầu tiên
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{}", addr);

    let request_count = Arc::new(AtomicUsize::new(0));
    let count_clone = request_count.clone();

    let handle = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let count = count_clone.fetch_add(1, Ordering::SeqCst) + 1;
            let _req = read_http_request(&mut stream).await;

            if count == 1 {
                // Lần 1: Treo kết nối lâu hơn timeout (1 giây) để kích hoạt socket timeout
                sleep(Duration::from_millis(1500)).await;
                let resp = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK";
                let _ = stream.write_all(resp.as_bytes()).await;
            } else {
                // Lần 2 (nếu có retry): Phản hồi thành công ngay lập tức
                let resp = "HTTP/1.1 200 OK\r\nContent-Length: 15\r\n\r\n{\"status\":\"ok\"}";
                let _ = stream.write_all(resp.as_bytes()).await;
            }
        }
    });

    let engine = CustomAgentApiEngine::new(&url, None);
    // Timeout 1 giây
    let input = create_test_input("test timeout", 1);
    let result = engine.execute(&input).await;

    // Kiểm tra hành vi khi chưa có cơ chế tự phục hồi (zero-retry):
    // 1. Kết quả trả về Err(EngineError::ApiRequestFailed) do client timeout
    assert!(result.is_err(), "Expected error due to socket timeout");
    match result.unwrap_err() {
        EngineError::ApiRequestFailed(err) => {
            let lower = err.to_lowercase();
            assert!(
                lower.contains("timeout") || lower.contains("timed out") || lower.contains("error sending request"),
                "Error should indicate timeout or send failure, got: {}",
                err
            );
        }
        other => panic!("Expected ApiRequestFailed, got {:?}", other),
    }

    // 2. Chỉ có đúng 1 request được gửi đi (không có retry tự phục hồi)
    assert_eq!(
        request_count.load(Ordering::SeqCst),
        1,
        "Engine should only attempt once without self-recovery"
    );

    handle.abort();
}

#[tokio::test]
async fn test_custom_api_intermittent_connection_reset_unrecovered() {
    // Mô phỏng server ngắt kết nối đột ngột (Connection Reset / EOF)
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{}", addr);

    let request_count = Arc::new(AtomicUsize::new(0));
    let count_clone = request_count.clone();

    let handle = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let count = count_clone.fetch_add(1, Ordering::SeqCst) + 1;
            let _req = read_http_request(&mut stream).await;

            if count == 1 {
                // Lần 1: Đóng socket đột ngột ngay sau khi nhận request mà không gửi response
                drop(stream);
            } else {
                // Lần 2 (nếu có retry): Phản hồi thành công
                let resp = "HTTP/1.1 200 OK\r\nContent-Length: 15\r\n\r\n{\"status\":\"ok\"}";
                let _ = stream.write_all(resp.as_bytes()).await;
            }
        }
    });

    let engine = CustomAgentApiEngine::new(&url, None);
    let input = create_test_input("test connection reset", 5);
    let result = engine.execute(&input).await;

    // Khi chưa có retry, request thất bại ngay lập tức
    assert!(result.is_err(), "Expected error due to connection reset");
    match result.unwrap_err() {
        EngineError::ApiRequestFailed(err) => {
            assert!(
                !err.is_empty(),
                "Error message should not be empty"
            );
        }
        other => panic!("Expected ApiRequestFailed, got {:?}", other),
    }

    // Chỉ thực hiện 1 lần thử duy nhất
    assert_eq!(
        request_count.load(Ordering::SeqCst),
        1,
        "Engine made no recovery attempts"
    );

    handle.abort();
}

#[tokio::test]
async fn test_custom_api_transient_io_broken_pipe_unrecovered() {
    // Mô phỏng Transient I/O: Server gửi dở header/body rồi ngắt kết nối
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{}", addr);

    let request_count = Arc::new(AtomicUsize::new(0));
    let count_clone = request_count.clone();

    let handle = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let count = count_clone.fetch_add(1, Ordering::SeqCst) + 1;
            let _req = read_http_request(&mut stream).await;

            if count == 1 {
                // Khai báo Content-Length 100 nhưng chỉ gửi 5 bytes rồi đóng
                let partial_resp = "HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\npart";
                let _ = stream.write_all(partial_resp.as_bytes()).await;
                let _ = stream.flush().await;
                drop(stream);
            } else {
                let resp = "HTTP/1.1 200 OK\r\nContent-Length: 15\r\n\r\n{\"status\":\"ok\"}";
                let _ = stream.write_all(resp.as_bytes()).await;
            }
        }
    });

    let engine = CustomAgentApiEngine::new(&url, None);
    let input = create_test_input("test transient io broken pipe", 5);
    let result = engine.execute(&input).await;

    assert!(result.is_err(), "Expected body reading error due to premature EOF");
    match result.unwrap_err() {
        EngineError::ApiRequestFailed(err) => {
            let lower = err.to_lowercase();
            assert!(
                lower.contains("body") || lower.contains("error") || lower.contains("eof") || lower.contains("closed"),
                "Error should mention body reading failure: {}",
                err
            );
        }
        other => panic!("Expected ApiRequestFailed, got {:?}", other),
    }

    assert_eq!(request_count.load(Ordering::SeqCst), 1);
    handle.abort();
}

#[tokio::test]
async fn test_openai_compat_intermittent_socket_timeout_unrecovered() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    let request_count = Arc::new(AtomicUsize::new(0));
    let count_clone = request_count.clone();

    let handle = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let count = count_clone.fetch_add(1, Ordering::SeqCst) + 1;
            let _req = read_http_request(&mut stream).await;

            if count == 1 {
                sleep(Duration::from_millis(1500)).await;
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n").await;
            } else {
                let json_body = serde_json::json!({
                    "choices": [{
                        "message": { "role": "assistant", "content": "Recovered" }
                    }]
                }).to_string();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    json_body.len(), json_body
                );
                let _ = stream.write_all(resp.as_bytes()).await;
            }
        }
    });

    let engine = OpenAiCompatibleEngine::new(&base_url, "api-key", "gpt-4o");
    let input = create_test_input("openai socket timeout test", 1);
    let result = engine.execute(&input).await;

    assert!(result.is_err(), "Expected error due to socket timeout");
    match result.unwrap_err() {
        EngineError::ApiRequestFailed(err) => {
            let lower = err.to_lowercase();
            assert!(
                lower.contains("timeout") || lower.contains("timed out") || lower.contains("error sending request"),
                "Error should indicate timeout or send failure, got: {}",
                err
            );
        }
        other => panic!("Expected ApiRequestFailed, got {:?}", other),
    }

    assert_eq!(
        request_count.load(Ordering::SeqCst),
        1,
        "OpenAI compat engine should not retry when unrecovered"
    );

    handle.abort();
}

#[tokio::test]
async fn test_openai_compat_intermittent_connection_reset_unrecovered() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    let request_count = Arc::new(AtomicUsize::new(0));
    let count_clone = request_count.clone();

    let handle = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let count = count_clone.fetch_add(1, Ordering::SeqCst) + 1;
            let _req = read_http_request(&mut stream).await;

            if count == 1 {
                // Drop stream to cause connection reset/closed
                drop(stream);
            } else {
                let json_body = serde_json::json!({
                    "choices": [{
                        "message": { "role": "assistant", "content": "Success" }
                    }]
                }).to_string();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    json_body.len(), json_body
                );
                let _ = stream.write_all(resp.as_bytes()).await;
            }
        }
    });

    let engine = OpenAiCompatibleEngine::new(&base_url, "api-key", "gpt-4o");
    let input = create_test_input("openai connection reset test", 5);
    let result = engine.execute(&input).await;

    assert!(result.is_err(), "Expected error due to connection reset");
    assert_eq!(
        request_count.load(Ordering::SeqCst),
        1,
        "Engine should only have attempted once"
    );

    handle.abort();
}

#[tokio::test]
async fn test_openai_compat_intermittent_503_service_unavailable_unrecovered() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    let request_count = Arc::new(AtomicUsize::new(0));
    let count_clone = request_count.clone();

    let handle = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let count = count_clone.fetch_add(1, Ordering::SeqCst) + 1;
            let _req = read_http_request(&mut stream).await;

            if count == 1 {
                let err_body = "{\"error\":{\"message\":\"Service Temporarily Unavailable\"}}";
                let resp = format!(
                    "HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    err_body.len(), err_body
                );
                let _ = stream.write_all(resp.as_bytes()).await;
            } else {
                let json_body = serde_json::json!({
                    "choices": [{
                        "message": { "role": "assistant", "content": "Recovered after 503" }
                    }]
                }).to_string();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    json_body.len(), json_body
                );
                let _ = stream.write_all(resp.as_bytes()).await;
            }
        }
    });

    let engine = OpenAiCompatibleEngine::new(&base_url, "api-key", "gpt-4o");
    let input = create_test_input("openai 503 test", 5);
    let result = engine.execute(&input).await;

    assert!(result.is_err(), "Expected 503 Service Unavailable error");
    match result.unwrap_err() {
        EngineError::ApiRequestFailed(err) => {
            assert!(
                err.contains("503"),
                "Error should report status 503, got: {}",
                err
            );
            assert!(
                err.contains("Service Temporarily Unavailable"),
                "Error should contain response body error: {}",
                err
            );
        }
        other => panic!("Expected ApiRequestFailed, got {:?}", other),
    }

    assert_eq!(
        request_count.load(Ordering::SeqCst),
        1,
        "Engine fails on 503 without retry in current implementation"
    );

    handle.abort();
}

#[tokio::test]
async fn test_openai_compat_intermittent_transient_io_corrupted_response_unrecovered() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    let request_count = Arc::new(AtomicUsize::new(0));
    let count_clone = request_count.clone();

    let handle = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let count = count_clone.fetch_add(1, Ordering::SeqCst) + 1;
            let _req = read_http_request(&mut stream).await;

            if count == 1 {
                // Trả về JSON bị cắt cụt (transient I/O parsing error)
                let corrupted = "{\"choices\": [{\"message\": {\"role\": \"assistant\", \"con";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    corrupted.len(), corrupted
                );
                let _ = stream.write_all(resp.as_bytes()).await;
            } else {
                let json_body = serde_json::json!({
                    "choices": [{
                        "message": { "role": "assistant", "content": "Valid JSON" }
                    }]
                }).to_string();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    json_body.len(), json_body
                );
                let _ = stream.write_all(resp.as_bytes()).await;
            }
        }
    });

    let engine = OpenAiCompatibleEngine::new(&base_url, "api-key", "gpt-4o");
    let input = create_test_input("openai corrupted json test", 5);
    let result = engine.execute(&input).await;

    assert!(result.is_err(), "Expected JSON parsing error");
    match result.unwrap_err() {
        EngineError::ApiRequestFailed(err) => {
            assert!(
                err.contains("parsing LLM JSON"),
                "Error should indicate JSON parse failure, got: {}",
                err
            );
        }
        other => panic!("Expected ApiRequestFailed, got {:?}", other),
    }

    assert_eq!(
        request_count.load(Ordering::SeqCst),
        1,
        "Engine fails immediately on JSON parse error without retry"
    );

    handle.abort();
}

#[tokio::test]
async fn test_resilient_custom_api_intermittent_connection_reset_recovered() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{}", addr);

    let request_count = Arc::new(AtomicUsize::new(0));
    let count_clone = request_count.clone();

    let handle = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let count = count_clone.fetch_add(1, Ordering::SeqCst) + 1;
            let _req = read_http_request(&mut stream).await;

            if count == 1 {
                // Drop connection abruptly on 1st attempt
                drop(stream);
            } else {
                let resp = "HTTP/1.1 200 OK\r\nContent-Length: 15\r\n\r\n{\"status\":\"ok\"}";
                let _ = stream.write_all(resp.as_bytes()).await;
            }
        }
    });

    let inner = CustomAgentApiEngine::new(&url, None);
    let resilient = ResilientApiEngine::new(inner, ApiFaultPolicy::none(), 3);
    let input = create_test_input("test resilient connection reset", 5);

    let output = resilient.execute(&input).await.expect("Resilient engine should recover");
    assert_eq!(output.exit_code, 0);
    assert!(output.stdout_content.contains("{\"status\":\"ok\"}"));
    assert_eq!(resilient.recovered_count(), 1);
    assert_eq!(resilient.resolved_count(), 1);
    assert_eq!(request_count.load(Ordering::SeqCst), 2);

    handle.abort();
}

#[tokio::test]
async fn test_resilient_openai_compat_intermittent_503_recovered() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    let request_count = Arc::new(AtomicUsize::new(0));
    let count_clone = request_count.clone();

    let handle = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let count = count_clone.fetch_add(1, Ordering::SeqCst) + 1;
            let _req = read_http_request(&mut stream).await;

            if count == 1 {
                let err_body = "{\"error\":{\"message\":\"Service Temporarily Unavailable\"}}";
                let resp = format!(
                    "HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    err_body.len(), err_body
                );
                let _ = stream.write_all(resp.as_bytes()).await;
            } else {
                let json_body = serde_json::json!({
                    "choices": [{
                        "message": { "role": "assistant", "content": "Recovered from 503 successfully" }
                    }]
                }).to_string();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    json_body.len(), json_body
                );
                let _ = stream.write_all(resp.as_bytes()).await;
            }
        }
    });

    let inner = OpenAiCompatibleEngine::new(&base_url, "test-key", "gpt-4o");
    let resilient = ResilientApiEngine::with_engine(inner);
    let input = create_test_input("test 503 recovery", 5);

    let output = resilient.execute(&input).await.expect("Resilient engine should recover from 503");
    assert_eq!(output.exit_code, 0);
    assert_eq!(output.stdout_content, "Recovered from 503 successfully");
    assert_eq!(resilient.recovered_count(), 1);
    assert_eq!(resilient.resolved_count(), 1);
    assert_eq!(request_count.load(Ordering::SeqCst), 2);

    handle.abort();
}

#[tokio::test]
async fn test_resilient_openai_compat_intermittent_transient_io_recovered() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    let request_count = Arc::new(AtomicUsize::new(0));
    let count_clone = request_count.clone();

    let handle = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let count = count_clone.fetch_add(1, Ordering::SeqCst) + 1;
            let _req = read_http_request(&mut stream).await;

            if count == 1 {
                let corrupted = "{\"choices\": [{\"message\": {\"role\": \"assistant\", \"con";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    corrupted.len(), corrupted
                );
                let _ = stream.write_all(resp.as_bytes()).await;
            } else {
                let json_body = serde_json::json!({
                    "choices": [{
                        "message": { "role": "assistant", "content": "Valid JSON Recovered" }
                    }]
                }).to_string();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    json_body.len(), json_body
                );
                let _ = stream.write_all(resp.as_bytes()).await;
            }
        }
    });

    let inner = OpenAiCompatibleEngine::new(&base_url, "test-key", "gpt-4o");
    let resilient = ResilientApiEngine::new(inner, ApiFaultPolicy::none(), 3);
    let input = create_test_input("test corrupted recovery", 5);

    let output = resilient.execute(&input).await.expect("Resilient engine should retry and recover");
    assert_eq!(output.stdout_content, "Valid JSON Recovered");
    assert_eq!(resilient.recovered_count(), 1);
    assert_eq!(resilient.resolved_count(), 1);
    assert_eq!(request_count.load(Ordering::SeqCst), 2);

    handle.abort();
}

#[tokio::test]
async fn test_resilient_api_chaos_injection_intermittent_timeout_recovery() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{}", addr);

    let handle = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let _req = read_http_request(&mut stream).await;
            let resp = "HTTP/1.1 200 OK\r\nContent-Length: 15\r\n\r\n{\"status\":\"ok\"}";
            let _ = stream.write_all(resp.as_bytes()).await;
        }
    });

    let inner = CustomAgentApiEngine::new(&url, None);
    // Injects 2 simulated socket timeouts before passing through to server
    let fault_policy = ApiFaultPolicy::always(ApiFault::SocketTimeout).with_max_injections(2);
    let resilient = ResilientApiEngine::new(inner, fault_policy, 3);
    let input = create_test_input("chaos policy test", 5);

    let output = resilient.execute(&input).await.expect("Resilient engine should recover from injected chaos");
    assert_eq!(output.exit_code, 0);
    assert_eq!(resilient.recovered_count(), 2);
    assert_eq!(resilient.resolved_count(), 1);

    handle.abort();
}

#[tokio::test]
async fn test_resilient_api_audit_ledger_recorded_flow() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{}", addr);

    let request_count = Arc::new(AtomicUsize::new(0));
    let count_clone = request_count.clone();

    let handle = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let count = count_clone.fetch_add(1, Ordering::SeqCst) + 1;
            let _req = read_http_request(&mut stream).await;

            if count == 1 {
                drop(stream);
            } else {
                let resp = "HTTP/1.1 200 OK\r\nContent-Length: 7\r\n\r\nSUCCESS";
                let _ = stream.write_all(resp.as_bytes()).await;
            }
        }
    });

    let inner = CustomAgentApiEngine::new(&url, None);
    let resilient = ResilientApiEngine::new(inner, ApiFaultPolicy::none(), 3);
    let input = create_test_input("audit ledger test", 5);
    let mut ledger = AuditLedger::new();

    let output = resilient
        .execute_and_record(&mut ledger, &input)
        .await
        .expect("Execution should succeed after retry");

    assert_eq!(output.stdout_content, "SUCCESS");
    assert_eq!(resilient.recovered_count(), 1);
    assert_eq!(resilient.resolved_count(), 1);
    // AuditLedger should record 1 retry attempt and 1 success event
    assert_eq!(ledger.len(), 2);

    handle.abort();
}

struct MockEngineWithFailures {
    failures_remaining: AtomicUsize,
    fail_error: EngineError,
}

#[async_trait::async_trait]
impl ComputeEngine for MockEngineWithFailures {
    async fn execute(&self, _input: &EngineInput) -> Result<EngineOutput, EngineError> {
        let rem = self.failures_remaining.load(Ordering::SeqCst);
        if rem > 0 {
            self.failures_remaining.fetch_sub(1, Ordering::SeqCst);
            Err(match &self.fail_error {
                EngineError::ApiRequestFailed(s) => EngineError::ApiRequestFailed(s.clone()),
                EngineError::Timeout => EngineError::Timeout,
                other => EngineError::ProcessFailed(other.to_string()),
            })
        } else {
            Ok(EngineOutput {
                stdout_content: "success".to_string(),
                exit_code: 0,
                execution_duration_ms: 5,
                engine_used: EngineKind::CustomAgentApi,
                session_id: None,
            })
        }
    }

    fn engine_kind(&self) -> EngineKind {
        EngineKind::CustomAgentApi
    }
}

#[tokio::test]
async fn test_fault_simulation_and_taxonomy() {
    assert_eq!(
        simulate_api_fault(&ApiFault::SocketTimeout),
        Err(ApiError::SocketTimeout)
    );
    assert_eq!(
        simulate_api_fault(&ApiFault::ConnectionReset),
        Err(ApiError::ConnectionReset)
    );
    assert!(matches!(
        simulate_api_fault(&ApiFault::RateLimitExceeded),
        Err(ApiError::RateLimitExceeded(_))
    ));
    assert!(matches!(
        simulate_api_fault(&ApiFault::TransientIo),
        Err(ApiError::TransientIo(_))
    ));
    assert!(matches!(
        simulate_api_fault(&ApiFault::CorruptedResponse),
        Err(ApiError::CorruptedResponse(_))
    ));
    assert!(matches!(
        simulate_api_fault(&ApiFault::ServiceUnavailable),
        Err(ApiError::ServiceUnavailable(_))
    ));
}

#[tokio::test]
async fn test_policy_intermittent_and_max_injections() {
    let mut policy =
        ApiFaultPolicy::intermittent(ApiFault::SocketTimeout, 2).with_max_injections(2);

    assert_eq!(policy.should_inject(), Some(ApiFault::SocketTimeout)); // inv 1
    assert_eq!(policy.should_inject(), None);                          // inv 2
    assert_eq!(policy.should_inject(), Some(ApiFault::SocketTimeout)); // inv 3
    assert_eq!(policy.should_inject(), None);                          // max reached
    assert_eq!(policy.injected_count(), 2);
}

#[tokio::test]
async fn test_resilient_engine_auto_recovers_transient_failures() {
    let inner = MockEngineWithFailures {
        failures_remaining: AtomicUsize::new(2),
        fail_error: EngineError::ApiRequestFailed("HTTP error: connection reset by peer".to_string()),
    };

    let resilient = ResilientApiEngine::new(inner, ApiFaultPolicy::none(), 3);
    let res = resilient.execute(&create_test_input("test", 5)).await;

    assert!(res.is_ok());
    assert_eq!(resilient.recovered_count(), 2);
    assert_eq!(resilient.resolved_count(), 1);
    assert_eq!(resilient.rejected_count(), 0);
}

#[tokio::test]
async fn test_resilient_engine_fails_when_exceeding_max_retries() {
    let inner = MockEngineWithFailures {
        failures_remaining: AtomicUsize::new(5),
        fail_error: EngineError::ApiRequestFailed("503 Service Unavailable".to_string()),
    };

    let resilient = ResilientApiEngine::new(inner, ApiFaultPolicy::none(), 2);
    let res = resilient.execute(&create_test_input("test", 5)).await;

    assert!(res.is_err());
    assert_eq!(resilient.recovered_count(), 2);
    assert_eq!(resilient.rejected_count(), 1);
    assert_eq!(resilient.resolved_count(), 0);
}

#[tokio::test]
async fn test_resilient_engine_chaos_injection_recovery() {
    let inner = MockEngineWithFailures {
        failures_remaining: AtomicUsize::new(0),
        fail_error: EngineError::Timeout,
    };

    // Injects 2 transient faults via policy, wrapper retries and recovers
    let policy = ApiFaultPolicy::always(ApiFault::SocketTimeout).with_max_injections(2);
    let resilient = ResilientApiEngine::new(inner, policy, 3);
    let res = resilient.execute(&create_test_input("test", 5)).await;

    assert!(res.is_ok());
    assert_eq!(resilient.recovered_count(), 2);
    assert_eq!(resilient.resolved_count(), 1);
}

#[tokio::test]
async fn test_resilient_engine_corrupted_response_not_retried() {
    let inner = MockEngineWithFailures {
        failures_remaining: AtomicUsize::new(0),
        fail_error: EngineError::Timeout,
    };

    // Corrupted response is fatal and rejected immediately
    let policy = ApiFaultPolicy::always(ApiFault::CorruptedResponse);
    let resilient = ResilientApiEngine::new(inner, policy, 3);
    let res = resilient.execute(&create_test_input("test", 5)).await;

    assert!(res.is_err());
    assert_eq!(resilient.rejected_count(), 1);
    assert_eq!(resilient.recovered_count(), 0);
}

#[tokio::test]
async fn test_execute_and_record_with_audit_ledger() {
    let inner = MockEngineWithFailures {
        failures_remaining: AtomicUsize::new(1),
        fail_error: EngineError::ApiRequestFailed("429 Too Many Requests".to_string()),
    };

    let resilient = ResilientApiEngine::new(inner, ApiFaultPolicy::none(), 3);
    let mut ledger = AuditLedger::new();
    let res = resilient
        .execute_and_record(&mut ledger, &create_test_input("test", 5))
        .await;

    assert!(res.is_ok());
    assert_eq!(resilient.recovered_count(), 1);
    assert_eq!(resilient.resolved_count(), 1);
    assert_eq!(ledger.len(), 2); // 1 retry attempt event + 1 success event
}

#[tokio::test]
async fn test_topic_0893f8_ad1a54_api_benchmark_and_resilience() {
    println!("[Reproduction & Benchmark Test] Executing TOPIC-20261105-0893f8 / ad1a54 API Benchmark...");

    // 1. Verify LOC compliance across all modular api components (< 350 lines P90)
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let api_dir = manifest_dir.join("src/api");
    assert!(
        api_dir.is_dir(),
        "src/api must exist as a modular directory"
    );

    let mut checked_files = 0;
    for entry in std::fs::read_dir(&api_dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            let content = std::fs::read_to_string(&path).unwrap();
            let lines = content.lines().count();
            assert!(
                lines <= 350,
                "API submodule {} has {} lines, which exceeds P90 limit (350 lines)",
                path.file_name().unwrap().to_str().unwrap(),
                lines
            );
            checked_files += 1;
        }
    }
    assert!(
        checked_files >= 4,
        "Expected at least 4 modular API source files, found {}",
        checked_files
    );

    // 2. High-load benchmark: 10,000 requests measuring P50/P90/P99 latency, throughput, and proving bounded memory allocation
    let inner_mock = BenchmarkMockApiEngine;
    let resilient = ResilientApiEngine::new(inner_mock, ApiFaultPolicy::none(), 3)
        .with_max_capacity(2_000);

    let stats: ApiBenchmarkStats = ApiBenchmarkHarness::run_benchmark(&resilient, 10_000).await;
    assert_eq!(stats.total_requests, 10_000);
    assert_eq!(stats.successful_resolved, 9_000);
    assert_eq!(stats.rejected_count, 1_000);
    assert!(
        stats.throughput_ops_per_sec > 500.0,
        "Throughput should exceed 500 ops/sec, got {}",
        stats.throughput_ops_per_sec
    );
    assert!(
        stats.p50_latency_us > 0.0,
        "P50 latency must be tracked"
    );
    assert!(
        stats.p90_latency_us > 0.0,
        "P90 latency must be tracked"
    );
    assert!(
        stats.p99_latency_us < 20_000.0,
        "P99 latency must be under 20ms, got {} us",
        stats.p99_latency_us
    );
    assert!(
        stats.memory_allocated_bytes > 0,
        "Memory allocation must be tracked"
    );

    // 3. Verify bounded memory allocation (no unbounded leak / FIFO eviction)
    let mem_before = resilient.memory_usage_bytes();
    // Run another 5,000 requests to trigger FIFO eviction in bounded capacity
    let _ = ApiBenchmarkHarness::run_benchmark(&resilient, 5_000).await;
    let mem_after = resilient.memory_usage_bytes();
    assert!(
        mem_after <= mem_before + 2048,
        "Memory usage must be strictly bounded under high load (before: {}, after: {})",
        mem_before,
        mem_after
    );

    // 4. Batch execution optimization test
    let batch_inputs = vec![
        create_test_input("valid_prompt_1", 5),
        create_test_input("valid_prompt_2", 5),
        create_test_input("invalid_batch_prompt", 5),
        create_test_input("valid_prompt_3", 5),
    ];
    let batch_res = resilient.execute_batch(&batch_inputs).await;
    assert_eq!(batch_res.len(), 4);
    assert!(batch_res[0].is_ok());
    assert!(batch_res[1].is_ok());
    assert!(matches!(batch_res[2], Err(EngineError::ApiRequestFailed(_))));
    assert!(batch_res[3].is_ok());

    // 5. Concurrent benchmark test (10,000 requests across 4 workers, measuring lock contention)
    let concurrent_stats = ApiBenchmarkHarness::run_concurrent_benchmark(10_000, 4).await;
    assert_eq!(concurrent_stats.total_requests, 10_000);
    assert_eq!(concurrent_stats.successful_resolved, 9_000);
    assert_eq!(concurrent_stats.rejected_count, 1_000);
    assert!(
        concurrent_stats.throughput_ops_per_sec > 500.0,
        "Concurrent throughput should exceed 500 ops/sec, got {}",
        concurrent_stats.throughput_ops_per_sec
    );
    assert!(
        concurrent_stats.p50_latency_us > 0.0,
        "Concurrent P50 latency should be measured"
    );
    assert!(
        concurrent_stats.p90_latency_us > 0.0,
        "Concurrent P90 latency should be measured"
    );
    assert!(
        concurrent_stats.p99_latency_us < 20_000.0,
        "Concurrent P99 latency should be well under 20ms, got {} us",
        concurrent_stats.p99_latency_us
    );
    assert!(
        concurrent_stats.memory_allocated_bytes > 0,
        "Concurrent memory usage should be tracked"
    );

    println!("[Reproduction & Benchmark Test] TOPIC-20261105-0893f8 / ad1a54 passed successfully!");
}

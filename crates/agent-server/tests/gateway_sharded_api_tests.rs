use std::sync::Arc;
use std::time::Duration;

use agent_engine::AccountEngineKind;
use agent_server::gateway::{
    GatewayBuffer, GatewayError, GatewayFaultPolicy, GatewayRingBuffer, ShardedGatewayCache,
    ShardedGatewayRingBuffer, ShardedResilientGatewayService,
};

#[test]
fn with_default_shards_creates_sixteen_shards() {
    let service = ShardedResilientGatewayService::with_default_shards();

    assert_eq!(service.num_shards(), 16);
    assert_eq!(service.total_capacity(), None);
    assert!(service.cache().is_none());
    assert!(service.cache_stats().is_none());
}

#[tokio::test]
async fn with_shared_cache_uses_supplied_cache_across_shards() {
    let cache = Arc::new(ShardedGatewayCache::new(4, 32));
    cache.insert(
        "preloaded-model".to_string(),
        (AccountEngineKind::Claude, Some("claude-test".to_string())),
    );
    let service = ShardedResilientGatewayService::new(4).with_shared_cache(cache.clone());

    assert!(Arc::ptr_eq(service.cache().unwrap(), &cache));
    assert_eq!(
        service.resolve_model_cached("preloaded-model").await,
        Ok((AccountEngineKind::Claude, Some("claude-test".to_string())))
    );
    let stats = service.cache_stats().unwrap();
    assert_eq!(stats.hits, 1);
    assert_eq!(stats.misses, 0);
    assert_eq!(stats.total_entries, 1);
}

#[tokio::test]
async fn with_history_buffer_shares_single_buffer_with_every_shard() {
    let history = Arc::new(GatewayRingBuffer::new(4));
    let service = ShardedResilientGatewayService::new(3).with_history_buffer(history.clone());

    assert_eq!(service.total_capacity(), Some(4));
    assert!(Arc::ptr_eq(&service.history_buffer().unwrap(), &history));
    assert!(service.sharded_history_buffer().is_none());
    assert!(
        matches!(service.buffer(), Some(GatewayBuffer::Single(buffer)) if Arc::ptr_eq(buffer, &history))
    );

    service
        .resolve_model_resilient("antigravity")
        .await
        .unwrap();
    assert_eq!(history.snapshot(), vec!["antigravity"]);
}

#[tokio::test]
async fn with_sharded_history_buffer_shares_sharded_buffer_with_every_shard() {
    let history = Arc::new(ShardedGatewayRingBuffer::new(3, 12));
    let service =
        ShardedResilientGatewayService::new(3).with_sharded_history_buffer(history.clone());

    assert_eq!(service.total_capacity(), Some(12));
    assert!(service.history_buffer().is_none());
    assert!(Arc::ptr_eq(
        &service.sharded_history_buffer().unwrap(),
        &history
    ));
    assert!(
        matches!(service.buffer(), Some(GatewayBuffer::Sharded(buffer)) if Arc::ptr_eq(buffer, &history))
    );

    service
        .resolve_model_resilient("antigravity")
        .await
        .unwrap();
    assert_eq!(history.snapshot().len(), 1);
    assert_eq!(history.total_pushed(), 1);
}

#[tokio::test]
async fn resolve_model_cached_caches_success_and_returns_invalid_model_error() {
    let service = ShardedResilientGatewayService::new(4).with_cache(32);

    let first = service
        .resolve_model_cached("gemini/gemini-2.0-flash-exp")
        .await;
    let second = service
        .resolve_model_cached("gemini/gemini-2.0-flash-exp")
        .await;
    assert_eq!(first, second);
    assert_eq!(
        first,
        Ok((
            AccountEngineKind::Antigravity,
            Some("gemini-2.0-flash-exp".to_string())
        ))
    );
    assert!(matches!(
        service.resolve_model_cached("invalid_engine_model").await,
        Err(GatewayError::InvalidModel(_))
    ));
    let stats = service.cache_stats().unwrap();
    assert_eq!(stats.hits, 1);
    assert_eq!(stats.misses, 2);
    assert_eq!(stats.total_entries, 1);
}

#[tokio::test]
async fn batch_resolve_models_preserves_order_and_individual_errors() {
    let service = ShardedResilientGatewayService::new(4).with_cache(32);
    let results = service
        .batch_resolve_models(&["codex/o1-preview", "not-a-model", "claude"])
        .await;

    assert_eq!(results.len(), 3);
    assert_eq!(
        results[0],
        Ok((AccountEngineKind::Codex, Some("o1-preview".to_string())))
    );
    assert!(matches!(results[1], Err(GatewayError::InvalidModel(_))));
    assert_eq!(results[2], Ok((AccountEngineKind::Claude, None)));

    let empty = service.batch_resolve_models(&[]).await;
    assert!(empty.is_empty());
}

#[tokio::test]
async fn execute_async_resilient_returns_operation_success_and_error() {
    let service = ShardedResilientGatewayService::new(4);
    let success = service
        .execute_async_resilient("success-key", || async { Ok::<_, GatewayError>(42) })
        .await;
    assert_eq!(success, Ok(42));

    let failure = service
        .execute_async_resilient("failure-key", || async {
            Err::<(), _>(GatewayError::EngineFailure("failed".to_string()))
        })
        .await;
    assert_eq!(
        failure,
        Err(GatewayError::EngineFailure("failed".to_string()))
    );
}

#[tokio::test]
async fn execute_async_resilient_retries_retryable_failures() {
    let service = ShardedResilientGatewayService::new(4)
        .with_fault_policy(GatewayFaultPolicy::none())
        .with_max_retries(1)
        .with_backoff(Duration::ZERO);
    let calls = std::sync::atomic::AtomicUsize::new(0);

    let result = service
        .execute_async_resilient("retry-key", || {
            let call = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async move {
                if call == 0 {
                    Err(GatewayError::ConnectionReset)
                } else {
                    Ok("recovered")
                }
            }
        })
        .await;

    assert_eq!(result, Ok("recovered"));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
}

#[test]
fn cache_stats_is_none_without_cache_and_reports_empty_cache_when_enabled() {
    assert!(
        ShardedResilientGatewayService::new(2)
            .cache_stats()
            .is_none()
    );

    let service = ShardedResilientGatewayService::new(2).with_cache(8);
    let stats = service.cache_stats().unwrap();
    assert_eq!(stats.hits, 0);
    assert_eq!(stats.misses, 0);
    assert_eq!(stats.total_entries, 0);
}

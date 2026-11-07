use std::sync::Arc;

use agent_engine::AccountEngineKind;
use agent_server::gateway::{
    GatewayBuffer, GatewayRingBuffer, ResilientGatewayService, ShardedGatewayCache,
    ShardedGatewayRingBuffer,
};

#[test]
fn gateway_resilience_configuration_is_empty_by_default() {
    let service = ResilientGatewayService::new();

    assert_eq!(service.max_capacity(), None);
    assert!(service.history_buffer().is_none());
    assert!(service.sharded_history_buffer().is_none());
    assert!(service.buffer().is_none());
}

#[test]
fn with_history_buffer_preserves_shared_buffer_and_capacity() {
    let history = Arc::new(GatewayRingBuffer::new(4));
    let service = ResilientGatewayService::new().with_history_buffer(history.clone());

    assert_eq!(service.max_capacity(), Some(4));
    assert!(Arc::ptr_eq(&service.history_buffer().unwrap(), &history));
    assert!(service.sharded_history_buffer().is_none());
    assert!(
        matches!(service.buffer(), Some(GatewayBuffer::Single(buffer)) if Arc::ptr_eq(buffer, &history))
    );

    service
        .resolve_model_resilient("antigravity")
        .expect("valid model should resolve");
    assert_eq!(history.snapshot(), vec!["antigravity"]);
}

#[test]
fn with_sharded_history_buffer_preserves_shared_buffer_and_capacity() {
    let history = Arc::new(ShardedGatewayRingBuffer::new(2, 8));
    let service = ResilientGatewayService::new().with_sharded_history_buffer(history.clone());

    assert_eq!(service.max_capacity(), Some(8));
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
        .expect("valid model should resolve");
    assert_eq!(history.snapshot(), vec!["antigravity"]);
}

#[test]
fn with_buffer_accepts_single_and_sharded_buffers() {
    let single = Arc::new(GatewayRingBuffer::new(3));
    let service = ResilientGatewayService::new().with_buffer(GatewayBuffer::Single(single.clone()));
    assert_eq!(service.max_capacity(), Some(3));
    assert!(Arc::ptr_eq(&service.history_buffer().unwrap(), &single));
    assert!(service.sharded_history_buffer().is_none());
    assert!(
        matches!(service.buffer(), Some(GatewayBuffer::Single(buffer)) if Arc::ptr_eq(buffer, &single))
    );

    let sharded = Arc::new(ShardedGatewayRingBuffer::new(2, 6));
    let service =
        ResilientGatewayService::new().with_buffer(GatewayBuffer::Sharded(sharded.clone()));
    assert_eq!(service.max_capacity(), Some(6));
    assert!(service.history_buffer().is_none());
    assert!(Arc::ptr_eq(
        &service.sharded_history_buffer().unwrap(),
        &sharded
    ));
    assert!(
        matches!(service.buffer(), Some(GatewayBuffer::Sharded(buffer)) if Arc::ptr_eq(buffer, &sharded))
    );
}

#[test]
fn with_shared_cache_uses_the_supplied_cache() {
    let cache = Arc::new(ShardedGatewayCache::new(2, 8));
    cache.insert("preloaded-model".into(), (AccountEngineKind::Claude, None));
    let service = ResilientGatewayService::new().with_shared_cache(cache.clone());

    assert!(Arc::ptr_eq(service.cache().unwrap(), &cache));
    assert_eq!(
        service.resolve_model_cached("preloaded-model"),
        Ok((AccountEngineKind::Claude, None))
    );
    assert_eq!(cache.stats().hits, 1);
}

#[test]
fn zero_capacity_history_is_reported_and_does_not_record_entries() {
    let history = Arc::new(GatewayRingBuffer::new(0));
    let service = ResilientGatewayService::new().with_history_buffer(history.clone());

    assert_eq!(service.max_capacity(), Some(0));
    service
        .resolve_model_resilient("antigravity")
        .expect("valid model should resolve");
    assert!(history.is_empty());
    assert_eq!(history.total_pushed(), 0);
}

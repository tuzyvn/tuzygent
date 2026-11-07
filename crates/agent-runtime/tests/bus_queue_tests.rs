use agent_domain::{AgentId, InterAgentEvent};
use agent_runtime::ShardedBusQueue;

fn event(from: &str, to: Option<&str>, event_type: &str) -> InterAgentEvent {
    InterAgentEvent::new(
        AgentId(from.to_string()),
        to.map(|id| AgentId(id.to_string())),
        event_type,
        serde_json::json!({"payload": "queue test"}),
    )
}

#[test]
fn shard_returns_existing_shards_and_rejects_out_of_range_indices() {
    let queue = ShardedBusQueue::new(4, 3);

    assert!(queue.shard(0).is_some());
    assert!(queue.shard(1).is_some());
    assert!(queue.shard(2).is_some());
    assert!(queue.shard(3).is_none());
    assert!(queue.shard(usize::MAX).is_none());

    // A zero shard request is normalized to one usable shard.
    let single_shard_queue = ShardedBusQueue::new(4, 0);
    assert!(single_shard_queue.shard(0).is_some());
    assert!(single_shard_queue.shard(1).is_none());
}

#[test]
fn shard_for_event_is_stable_in_range_and_uses_single_shard_boundary() {
    let queue = ShardedBusQueue::new(4, 7);
    let first = event("sender-a", Some("recipient"), "task.created");
    let same_route = event("sender-b", Some("recipient"), "other.event");

    let shard = queue.shard_for_event(&first);
    assert!(shard < queue.shard_count());
    assert_eq!(shard, queue.shard_for_event(&first));
    assert_eq!(shard, queue.shard_for_event(&same_route));

    let single_shard_queue = ShardedBusQueue::new(4, 0);
    assert_eq!(single_shard_queue.shard_count(), 1);
    assert_eq!(single_shard_queue.shard_for_event(&first), 0);
}

#[tokio::test]
async fn publish_delivers_to_event_shard_and_reports_boundary_results() {
    let queue = ShardedBusQueue::new(4, 5);
    let event = event("sender", Some("recipient"), "task.created");
    let target_shard = queue.shard_for_event(&event);
    let mut target_receiver = queue.subscribe_shard(target_shard).unwrap();
    let mut other_receiver = queue
        .subscribe_shard((target_shard + 1) % queue.shard_count())
        .unwrap();

    assert_eq!(queue.publish(event.clone()).unwrap(), 1);
    assert_eq!(target_receiver.recv().await.unwrap(), event);
    assert!(other_receiver.try_recv().is_err());

    let no_subscribers = ShardedBusQueue::new(2, 2);
    assert_eq!(no_subscribers.publish(event.clone()).unwrap(), 0);

    queue.close_all();
    assert!(queue.publish(event).is_err());
}

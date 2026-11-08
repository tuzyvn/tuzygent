use agent_domain::{AgentId, InterAgentEvent};
use agent_runtime::InterAgentBus;

fn event(event_type: &str) -> InterAgentEvent {
    InterAgentEvent::new(
        AgentId::new("sender"),
        None,
        event_type,
        serde_json::json!({"event_type": event_type}),
    )
}

#[tokio::test]
async fn publish_batch_reports_and_delivers_each_event_to_every_subscriber() {
    let bus = InterAgentBus::new(8);
    let mut first = bus.subscribe();
    let mut second = bus.subscribe();
    let events = vec![event("first"), event("second")];
    let expected = events.clone();

    assert_eq!(bus.publish_batch(events).unwrap(), 2);

    for event in expected {
        assert_eq!(first.recv().await.unwrap(), event);
        assert_eq!(second.recv().await.unwrap(), event);
    }
}

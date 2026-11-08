use agent_domain::{AgentId, InterAgentEvent};
use agent_runtime::{BusError, ResilientSubscriber};
use tokio::sync::broadcast;

fn event() -> InterAgentEvent {
    InterAgentEvent::new(
        AgentId::new("sender"),
        Some(AgentId::new("recipient")),
        "task.created",
        serde_json::json!({"payload": "subscriber test"}),
    )
}

#[test]
fn try_recv_resilient_reports_event_empty_and_closed_states() {
    let (sender, receiver) = broadcast::channel(4);
    let mut subscriber = ResilientSubscriber::new(receiver);

    assert_eq!(subscriber.try_recv_resilient(), Ok(None));

    let expected = event();
    sender.send(expected.clone()).unwrap();
    assert_eq!(subscriber.try_recv_resilient(), Ok(Some(expected)));

    drop(sender);
    assert_eq!(
        subscriber.try_recv_resilient(),
        Err(BusError::ChannelClosed)
    );
}

#[test]
fn inner_mut_allows_raw_receive_and_advances_subscriber_receiver() {
    let (sender, receiver) = broadcast::channel(4);
    let mut subscriber = ResilientSubscriber::new(receiver);
    let expected = event();

    sender.send(expected.clone()).unwrap();
    assert_eq!(subscriber.inner_mut().try_recv(), Ok(expected));
    assert_eq!(subscriber.try_recv_resilient(), Ok(None));
}

#[tokio::test]
async fn disabling_auto_catchup_reports_and_tracks_missed_messages() {
    let (sender, receiver) = broadcast::channel(2);
    let mut subscriber = ResilientSubscriber::new(receiver).with_auto_catchup(false);

    for _ in 0..5 {
        sender.send(event()).unwrap();
    }

    assert_eq!(
        subscriber.recv_resilient().await,
        Err(BusError::ReceiverLagged(3))
    );
    assert_eq!(subscriber.total_lagged(), 3);
}

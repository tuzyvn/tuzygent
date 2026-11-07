use agent_cli::commands::demo::{comedy_show, multi_agent};

#[tokio::test]
async fn multi_agent_reports_an_unknown_engine() {
    let error = multi_agent("definitely-not-a-configured-engine".to_string(), ".".to_string())
        .await
        .expect_err("an unknown engine should be rejected before starting the demo");

    assert!(error
        .to_string()
        .contains("Unknown engine or unsupported engine kind"));
}

#[tokio::test]
async fn comedy_show_completes_when_the_local_engine_is_unavailable() {
    let result = comedy_show("omniroute".to_string()).await;

    assert!(
        result.is_ok(),
        "engine request failures are handled by ending the show gracefully"
    );
}

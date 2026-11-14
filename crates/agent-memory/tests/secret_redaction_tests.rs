use agent_memory::{DurableMemoryStore, ScratchpadManager, StructuredLesson};
use tokio::fs;

#[tokio::test]
async fn durable_memory_store_redacts_secrets_before_persisting() -> anyhow::Result<()> {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_redact_{}",
        uuid::Uuid::new_v4().simple()
    ));
    let store = DurableMemoryStore::new(&temp_dir);

    let fake_openai = format!("{}{}", "sk-ant-", "abcdefghijklmnopqrstuvwxyz12");
    let fake_gh = format!("{}{}", "ghp_", "1234567890abcdefghijklmnop");
    let fake_url = "https://deploy:secretpass99@registry.local/pkg";

    store
        .save_lesson(&format!("Avoid leaking {fake_openai} in logs"))
        .await?;
    store
        .save_user_directive(&format!("Use token-free env instead of --token {fake_gh}"))
        .await?;

    let structured = StructuredLesson {
        id: "les-sec-01".to_string(),
        timestamp: "2026-11-14T00:00:00Z".to_string(),
        task_title: format!("Fix upload with {fake_url}"),
        trigger_error: Some(format!("401 using Authorization: Bearer {fake_openai}")),
        remediation_applied: Some(format!("Removed api_key={fake_gh}")),
        general_insight: format!("Do not embed {fake_openai} anywhere"),
        success: true,
    };
    store.save_structured_lesson(&structured).await?;

    let memory_md = fs::read_to_string(store.memory_file()).await?;
    assert!(!memory_md.contains(&fake_openai));
    assert!(!memory_md.contains(&fake_gh));
    assert!(!memory_md.contains("secretpass99"));
    assert!(memory_md.contains("«redacted "));

    let user_md = fs::read_to_string(store.user_file()).await?;
    assert!(!user_md.contains(&fake_gh));
    assert!(user_md.contains("«redacted "));

    let lessons = store.load_structured_lessons().await?;
    assert_eq!(lessons.len(), 1);
    let saved = &lessons[0];
    assert!(!saved.task_title.contains("secretpass99"));
    assert!(!saved
        .trigger_error
        .as_deref()
        .unwrap()
        .contains(&fake_openai));
    assert!(!saved
        .remediation_applied
        .as_deref()
        .unwrap()
        .contains(&fake_gh));
    assert!(!saved.general_insight.contains(&fake_openai));

    let _ = fs::remove_dir_all(&temp_dir).await;
    Ok(())
}

#[tokio::test]
async fn scratchpad_manager_redacts_secrets_on_write_and_append_log() -> anyhow::Result<()> {
    let temp_dir = std::env::temp_dir().join(format!(
        "scratchpad_redact_{}",
        uuid::Uuid::new_v4().simple()
    ));
    let pad = ScratchpadManager::for_task(&temp_dir, "task-secret-check");
    let fake_token = format!("{}{}", "xai-", "1234567890abcdefghijklmnop");

    pad.write(&format!("Initial note with secret {fake_token}\n"))
        .await?;
    pad.append_log(&format!("Completed step with API_KEY={fake_token}"))
        .await?;

    let content = pad.read().await?;
    assert!(!content.contains(&fake_token));
    assert!(content.contains("«redacted "));

    let _ = fs::remove_dir_all(&temp_dir).await;
    Ok(())
}

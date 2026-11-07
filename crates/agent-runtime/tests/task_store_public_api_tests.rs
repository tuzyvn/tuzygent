use agent_domain::{Priority, Task, TaskStatus};
use agent_runtime::{SqliteTaskStore, TaskStore};

#[tokio::test]
async fn task_store_public_api_persists_and_claims_queued_task() {
    let workspace = std::env::temp_dir().join(format!(
        "agent-runtime-task-store-public-api-{}",
        uuid::Uuid::new_v4()
    ));
    let task = Task::new(
        "public API task",
        "verify task store behavior",
        Priority::Normal,
    );
    let task_id = task.id.clone();

    {
        let store = SqliteTaskStore::new(&workspace).expect("open task store");
        TaskStore::save_task(&store, &task)
            .await
            .expect("save task through TaskStore");
        assert_eq!(TaskStore::count_tasks(&store).await.unwrap(), 1);
    }

    let store = SqliteTaskStore::new(&workspace).expect("reopen task store");
    let persisted = TaskStore::get_task(&store, &task_id)
        .await
        .unwrap()
        .expect("persisted task should be readable");
    assert_eq!(persisted.id, task.id);
    assert_eq!(persisted.title, task.title);
    assert_eq!(persisted.goal_description, task.goal_description);
    assert_eq!(persisted.status, TaskStatus::Queued);

    let claimed = TaskStore::claim_queued(&store, &task_id)
        .await
        .expect("claim queued task")
        .expect("task should be claimable");
    assert_eq!(claimed.id, task_id);
    assert_eq!(claimed.status, TaskStatus::Executing);
    assert!(TaskStore::claim_queued(&store, &task_id)
        .await
        .unwrap()
        .is_none());

    drop(store);
    std::fs::remove_dir_all(workspace).expect("remove temporary task store");
}

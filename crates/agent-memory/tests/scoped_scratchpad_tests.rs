use agent_memory::{ScopeAccess, ScopedScratchpad, ScratchpadManager};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(500);

fn unique_temp_dir() -> PathBuf {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let ts = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("agentcore_scoped_pad_{}_{}", ts, id));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

#[tokio::test]
async fn test_readonly_scoped_scratchpad_blocks_writes() {
    let ws = unique_temp_dir();
    let root_mgr = ScratchpadManager::for_task(&ws, "parent-task-1");
    root_mgr.append_log("Root plan initialized").await.unwrap();

    let child_scope = root_mgr.scoped_child("subagent-auditor", ScopeAccess::ReadOnly);
    assert_eq!(child_scope.scope_id(), "subagent-auditor");
    assert_eq!(child_scope.access(), ScopeAccess::ReadOnly);

    // Reading parent context should succeed
    let parent_content = child_scope.read_parent().await.unwrap();
    assert!(parent_content.contains("Root plan initialized"));

    // Attempting to write or append log to a ReadOnly scope must be rejected
    let write_res = child_scope.write("Unauthorized overwrite attempt").await;
    assert!(write_res.is_err(), "ReadOnly scratchpad must reject write");
    assert_eq!(
        write_res.unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );

    let append_res = child_scope.append_log("Unauthorized log attempt").await;
    assert!(
        append_res.is_err(),
        "ReadOnly scratchpad must reject append_log"
    );
    assert_eq!(
        append_res.unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );

    // Parent scratchpad remains pristine
    let root_content = root_mgr.read().await.unwrap();
    assert!(!root_content.contains("Unauthorized"));

    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn test_readwrite_scoped_scratchpad_isolation_and_merge() {
    let ws = unique_temp_dir();
    let root_mgr = ScratchpadManager::for_task(&ws, "parent-task-2");
    root_mgr
        .append_log("Parent starting sub-delegation")
        .await
        .unwrap();

    let child_scope_1 = root_mgr.scoped_child("subagent-worker-1", ScopeAccess::ReadWrite);
    let child_scope_2 = root_mgr.scoped_child("subagent-worker-2", ScopeAccess::ReadWrite);

    // Each child writes to their own isolated workspace
    child_scope_1
        .append_log("Worker 1 step A completed")
        .await
        .unwrap();
    child_scope_2
        .append_log("Worker 2 step B completed")
        .await
        .unwrap();

    let content_1 = child_scope_1.read().await.unwrap();
    let content_2 = child_scope_2.read().await.unwrap();

    // Verify isolation between siblings
    assert!(content_1.contains("Worker 1 step A"));
    assert!(!content_1.contains("Worker 2 step B"));
    assert!(content_2.contains("Worker 2 step B"));
    assert!(!content_2.contains("Worker 1 step A"));

    // Parent has not yet received child logs directly
    let root_before_merge = root_mgr.read().await.unwrap();
    assert!(!root_before_merge.contains("Worker 1 step A"));

    // Merge child 1 outcome into parent
    child_scope_1
        .merge_into_parent("Completed database schema migration safely")
        .await
        .unwrap();

    let root_after_merge = root_mgr.read().await.unwrap();
    assert!(root_after_merge.contains("[DELEGATION RESULT: subagent-worker-1]"));
    assert!(root_after_merge.contains("Completed database schema migration safely"));

    // Clean up child scopes explicitly
    child_scope_1.cleanup().await.unwrap();
    assert!(!child_scope_1.file_path().exists());

    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn test_hierarchical_nesting_and_auto_cleanup() {
    let ws = unique_temp_dir();
    let root_scope = ScopedScratchpad::new(&ws, "root-agent", ScopeAccess::ReadWrite);
    root_scope.append_log("Level 0 initiated").await.unwrap();

    // Child scope
    let child = root_scope.child_scope("sub-agent-l1", ScopeAccess::ReadWrite);
    child.append_log("Level 1 running").await.unwrap();

    // Grandchild scope with auto_cleanup enabled
    let grandchild_path;
    {
        let grandchild = child
            .child_scope("sub-agent-l2", ScopeAccess::ReadWrite)
            .with_auto_cleanup(true);
        grandchild.append_log("Level 2 execution").await.unwrap();
        grandchild_path = grandchild.file_path().to_path_buf();
        assert!(grandchild_path.exists());
        // grandchild drops here, triggering auto-cleanup
    }
    assert!(
        !grandchild_path.exists(),
        "Grandchild scratchpad should be auto-cleaned upon drop"
    );

    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn test_subagent_scoped_scratchpad_permissions_isolation_anti_snooping_and_cleanup() {
    let ws = unique_temp_dir();
    let root_mgr = ScratchpadManager::for_task(&ws, "root-orchestrator-task");
    root_mgr
        .append_log("ROOT_SECRET_TOKEN=xyz123; CONFIDENTIAL_PARENT_STRATEGY=Alpha")
        .await
        .unwrap();

    // 1. Phân quyền ReadOnly cho Auditor Sub-Agent: Được đọc cha nhưng cấm ghi ô nhiễm
    let auditor = root_mgr.scoped_child("auditor-agent", ScopeAccess::ReadOnly);
    assert_eq!(auditor.access(), ScopeAccess::ReadOnly);
    let parent_text = auditor.read_parent().await.unwrap();
    assert!(parent_text.contains("CONFIDENTIAL_PARENT_STRATEGY=Alpha"));

    let err_write = auditor
        .write("Auditor illegal overwrite")
        .await
        .unwrap_err();
    assert_eq!(err_write.kind(), std::io::ErrorKind::PermissionDenied);
    let err_append = auditor
        .append_log("Auditor illegal append")
        .await
        .unwrap_err();
    assert_eq!(err_append.kind(), std::io::ErrorKind::PermissionDenied);

    // 2. Phân quyền Isolated: Cô lập hoàn toàn, cấm đọc trộm bộ nhớ cha (Anti-Snooping)
    let isolated_worker = root_mgr.isolated_child("external-worker");
    assert_eq!(isolated_worker.access(), ScopeAccess::Isolated);

    // Đọc trộm bộ nhớ cha bị từ chối dứt khoát
    let snoop_attempt = isolated_worker.read_parent().await;
    assert!(
        snoop_attempt.is_err(),
        "Isolated sub-agent must be prevented from reading parent memory"
    );
    assert_eq!(
        snoop_attempt.unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );

    // Ghi cục bộ độc lập trong vùng nhớ của isolated_worker
    isolated_worker
        .append_log("Processing external job #99 without parent secrets")
        .await
        .unwrap();
    assert!(isolated_worker
        .read()
        .await
        .unwrap()
        .contains("Processing external job #99"));

    // Kiểm tra bộ nhớ cha không bị ô nhiễm bởi các log cục bộ của isolated_worker
    let parent_after_worker = root_mgr.read().await.unwrap();
    assert!(!parent_after_worker.contains("Processing external job #99"));

    // 3. Cô lập giữa các Sub-Agent ngang hàng (Sibling Isolation)
    let sibling_a = root_mgr.scoped_child("worker-node-a", ScopeAccess::ReadWrite);
    let sibling_b = root_mgr.scoped_child("worker-node-b", ScopeAccess::ReadWrite);

    sibling_a
        .append_log("Internal state of worker A")
        .await
        .unwrap();
    sibling_b
        .append_log("Internal state of worker B")
        .await
        .unwrap();

    let text_a = sibling_a.read().await.unwrap();
    let text_b = sibling_b.read().await.unwrap();
    assert!(text_a.contains("worker A") && !text_a.contains("worker B"));
    assert!(text_b.contains("worker B") && !text_b.contains("worker A"));

    // 4. Tự động thu gom / dọn dẹp khi Drop (Auto-Cleanup)
    let temp_child_path;
    {
        let auto_clean_child = sibling_a
            .child_scope("ephemeral-sub-step", ScopeAccess::ReadWrite)
            .with_auto_cleanup(true);
        auto_clean_child
            .append_log("Ephemeral intermediate calculation")
            .await
            .unwrap();
        temp_child_path = auto_clean_child.file_path().to_path_buf();
        assert!(temp_child_path.exists());
    }
    assert!(
        !temp_child_path.exists(),
        "Temporary sub-agent scratchpad must be cleaned up on drop"
    );

    // 5. Gộp kết quả có kiểm soát (Controlled Merge) vào parent khi hoàn tất
    sibling_a
        .merge_into_parent("Node A calculation finished with status SUCCESS (Checksum: 0x98AF)")
        .await
        .unwrap();

    let parent_after_merge = root_mgr.read().await.unwrap();
    assert!(parent_after_merge.contains("[DELEGATION RESULT: worker-node-a]"));
    assert!(parent_after_merge.contains("Checksum: 0x98AF"));
    // Đảm bảo không bị ô nhiễm bởi các log chi tiết nội bộ của worker A
    assert!(!parent_after_merge.contains("Internal state of worker A"));

    // Thu dọn thủ công tệp của các worker còn lại
    sibling_a.cleanup().await.unwrap();
    sibling_b.cleanup().await.unwrap();
    isolated_worker.cleanup().await.unwrap();
    assert!(!sibling_a.file_path().exists());
    assert!(!sibling_b.file_path().exists());
    assert!(!isolated_worker.file_path().exists());

    let _ = std::fs::remove_dir_all(&ws);
}

use agent_memory::DurableMemoryStore;
use std::path::PathBuf;

#[test]
fn legacy_lessons_file_returns_expected_path_without_creating_storage() {
    let relative_workspace = PathBuf::from(format!(
        "relative-workspace-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let relative_expected = relative_workspace
        .join("storage")
        .join("harness_memory")
        .join("lessons.json");
    let relative_store = DurableMemoryStore::new(&relative_workspace);

    assert_eq!(relative_store.legacy_lessons_file(), relative_expected);
    assert!(
        !relative_workspace.exists(),
        "retrieving the path must not create the relative workspace"
    );

    let absolute_workspace = std::env::temp_dir().join(format!(
        "absolute-workspace-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let absolute_expected = absolute_workspace
        .join("storage")
        .join("harness_memory")
        .join("lessons.json");
    let absolute_store = DurableMemoryStore::new(&absolute_workspace);

    assert_eq!(absolute_store.legacy_lessons_file(), absolute_expected);
    assert!(
        !absolute_workspace.exists(),
        "retrieving the path must not create the absolute workspace"
    );
}

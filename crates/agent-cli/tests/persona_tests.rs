use agent_cli::commands::persona::init_samples;
use agent_memory::PersonaLoader;

#[tokio::test]
async fn init_samples_writes_loadable_sample_personas() {
    let output_dir = std::env::temp_dir().join(format!(
        "agent-cli-persona-samples-{}",
        uuid::Uuid::new_v4()
    ));

    init_samples(output_dir.to_string_lossy().into_owned())
        .await
        .expect("sample personas should be initialized");

    let devops = PersonaLoader::load_from_dir(output_dir.join("devops-sre"))
        .await
        .expect("DevOps sample should be loadable");
    assert_eq!(devops.identity.id.0, "agent-sre-teo");
    assert_eq!(devops.identity.name, "Tèo SRE DevOps");

    let auditor = PersonaLoader::load_from_dir(output_dir.join("financial-auditor"))
        .await
        .expect("financial auditor sample should be loadable");
    assert_eq!(auditor.identity.id.0, "agent-acdoca-auditor");
    assert_eq!(auditor.identity.name, "Mai Financial Auditor");

    std::fs::remove_dir_all(output_dir).expect("temporary sample directory should be removable");
}

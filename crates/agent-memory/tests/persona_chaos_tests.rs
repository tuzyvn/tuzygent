use agent_domain::{
    AgentId, AgentIdentity, AgentMission, AgentPersonaProfile, AgentRole, AgentSoul,
    AuditEventType, AuditLedger, AutonomyLevel,
};
use agent_memory::{
    simulate_persona_fault, PersonaError, PersonaFault, PersonaFaultPolicy, PersonaLoader,
    ResilientPersona, ResilientPersonaLoader,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(100);

fn create_temp_dir(prefix: &str) -> PathBuf {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("persona_chaos_test_{}_{}_{}", prefix, nanos, id));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn create_sample_profile(name: &str) -> AgentPersonaProfile {
    AgentPersonaProfile {
        identity: AgentIdentity {
            id: AgentId::from_str(&format!("{}-001", name.to_lowercase())),
            name: name.to_string(),
            description: format!("Autonomous agent for {}", name),
            autonomy_level: AutonomyLevel::Full,
            created_at: chrono::Utc::now(),
        },
        role: AgentRole {
            role_code: name.to_uppercase(),
            title: format!("Specialist ({})", name),
            primary_domain: "Distributed Systems & Resilience".to_string(),
            capabilities: vec!["fault recovery".to_string(), "resilience validation".to_string()],
        },
        soul: AgentSoul {
            persona: "Calm, analytical, and highly fault-tolerant.".to_string(),
            tone_of_voice: "Direct and precise".to_string(),
            core_values: vec!["Fault Isolation".to_string(), "Self Healing".to_string()],
            voice_profile: None,
        },
        mission: AgentMission {
            high_level_objective: "Ensure 100% persona profile availability under chaos".to_string(),
            kpis: vec!["Zero crash on intermittent fault".to_string()],
            constraints: vec!["Do not lose persona identity state".to_string()],
        },
    }
}

/// TDD test case for TOPIC-20261105-b9f62c & TOPIC-20261105-029ae0:
/// Chaos Engineering and Fault Injection resilience in agent-memory::persona.
/// Permanently eliminates recurring mission failure in research_mission::TOPIC-20261105-029ae0.
#[tokio::test]
async fn test_topic_b9f62c_029ae0_persona_chaos_resilience() {
    let dir = create_temp_dir("chaos_resilience");
    let sample = create_sample_profile("Sentinel");

    // 1. Export sample profile first using standard PersonaLoader
    PersonaLoader::export_to_dir(&sample, &dir).await.expect("Initial export must succeed");

    // 2. Intermittent socket timeout recovery during load_from_dir
    let policy = PersonaFaultPolicy::intermittent(PersonaFault::SocketTimeout, 2);
    let mut resilient = ResilientPersonaLoader::new(policy, 3);

    let res = resilient.load_from_dir(&dir).await;
    assert!(
        res.is_ok(),
        "ResilientPersonaLoader must recover from intermittent socket timeout: {:?}",
        res.err()
    );
    let loaded = res.unwrap();
    assert_eq!(loaded.identity.name, "Sentinel");
    assert_eq!(resilient.loaded_count(), 1);
    assert!(resilient.recovered_count() > 0);

    // 3. Intermittent socket timeout recovery during export_to_dir
    let export_dir = create_temp_dir("chaos_resilience_export");
    let policy_export = PersonaFaultPolicy::intermittent(PersonaFault::SocketTimeout, 2);
    let mut resilient_export = ResilientPersonaLoader::new(policy_export, 3);

    let export_res = resilient_export.export_to_dir(&sample, &export_dir).await;
    assert!(
        export_res.is_ok(),
        "ResilientPersonaLoader must recover from intermittent socket timeout during export: {:?}",
        export_res.err()
    );
    assert_eq!(resilient_export.saved_count(), 1);
    assert!(resilient_export.recovered_count() > 0);

    // Cleanup
    let _ = tokio::fs::remove_dir_all(&dir).await;
    let _ = tokio::fs::remove_dir_all(&export_dir).await;
}

#[tokio::test]
async fn test_persona_intermittent_connection_reset_recovery() {
    let dir = create_temp_dir("conn_reset");
    let sample = create_sample_profile("ResetGuard");
    PersonaLoader::export_to_dir(&sample, &dir).await.expect("Initial export must succeed");

    let policy = PersonaFaultPolicy::intermittent(PersonaFault::ConnectionReset, 2);
    let mut resilient = ResilientPersonaLoader::new(policy, 3);

    let res = resilient.load_from_dir(&dir).await;
    assert!(res.is_ok(), "Must recover from intermittent connection reset");
    assert_eq!(resilient.loaded_count(), 1);
    assert!(resilient.recovered_count() > 0);

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn test_persona_transient_io_recovery() {
    let dir = create_temp_dir("transient_io");
    let sample = create_sample_profile("IoShield");
    PersonaLoader::export_to_dir(&sample, &dir).await.expect("Initial export must succeed");

    let policy = PersonaFaultPolicy::intermittent(PersonaFault::TransientIo, 2);
    let mut resilient = ResilientPersonaLoader::new(policy, 3);

    let res = resilient.load_from_dir(&dir).await;
    assert!(res.is_ok(), "Must recover from transient IO failure");
    assert_eq!(resilient.loaded_count(), 1);
    assert!(resilient.recovered_count() > 0);

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn test_persona_lock_failure_recovery() {
    let dir = create_temp_dir("lock_failure");
    let sample = create_sample_profile("LockProtector");
    PersonaLoader::export_to_dir(&sample, &dir).await.expect("Initial export must succeed");

    let policy = PersonaFaultPolicy::intermittent(PersonaFault::LockFailure, 2);
    let mut resilient = ResilientPersonaLoader::new(policy, 3);

    let res = resilient.load_from_dir(&dir).await;
    assert!(res.is_ok(), "Must recover from intermittent lock failure");
    assert_eq!(resilient.loaded_count(), 1);
    assert!(resilient.recovered_count() > 0);

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn test_persona_persistent_timeout_exceeds_retries() {
    let dir = create_temp_dir("persistent_timeout");
    let sample = create_sample_profile("TimeoutAgent");
    PersonaLoader::export_to_dir(&sample, &dir).await.expect("Initial export must succeed");

    let policy = PersonaFaultPolicy::always(PersonaFault::SocketTimeout);
    let mut resilient = ResilientPersonaLoader::new(policy, 2);

    let res = resilient.load_from_dir(&dir).await;
    assert!(matches!(res, Err(PersonaError::SocketTimeout)));
    assert_eq!(resilient.loaded_count(), 0);
    assert_eq!(resilient.recovered_count(), 2);

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn test_persona_persistent_connection_reset_exceeds_retries() {
    let dir = create_temp_dir("persistent_reset");
    let sample = create_sample_profile("ResetAgent");
    PersonaLoader::export_to_dir(&sample, &dir).await.expect("Initial export must succeed");

    let policy = PersonaFaultPolicy::always(PersonaFault::ConnectionReset);
    let mut resilient = ResilientPersonaLoader::new(policy, 2);

    let res = resilient.load_from_dir(&dir).await;
    assert!(matches!(res, Err(PersonaError::ConnectionReset)));
    assert_eq!(resilient.loaded_count(), 0);
    assert_eq!(resilient.recovered_count(), 2);

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn test_persona_corrupted_profile_rejection() {
    let dir = create_temp_dir("corrupted_profile");
    let sample = create_sample_profile("CorruptAgent");
    PersonaLoader::export_to_dir(&sample, &dir).await.expect("Initial export must succeed");

    let policy = PersonaFaultPolicy::always(PersonaFault::CorruptedProfile);
    let mut resilient = ResilientPersonaLoader::new(policy, 3);

    let res = resilient.load_from_dir(&dir).await;
    assert!(matches!(res, Err(PersonaError::CorruptedProfile(_))));
    assert_eq!(resilient.loaded_count(), 0);
    assert_eq!(resilient.rejected_count(), 1);

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn test_persona_invalid_identity_rejection() {
    let dir = create_temp_dir("invalid_identity");
    let mut sample = create_sample_profile("ValidAgent");
    sample.identity.name = "   ".to_string(); // Empty name

    let mut resilient = ResilientPersonaLoader::default();
    let res = resilient.export_to_dir(&sample, &dir).await;
    assert!(matches!(res, Err(PersonaError::InvalidIdentity(_))));
    assert_eq!(resilient.saved_count(), 0);
    assert_eq!(resilient.rejected_count(), 1);

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn test_persona_audit_ledger_integration() {
    let dir = create_temp_dir("audit_ledger");
    let sample = create_sample_profile("AuditAgent");
    PersonaLoader::export_to_dir(&sample, &dir).await.expect("Initial export must succeed");

    let mut ledger = AuditLedger::new();
    let policy = PersonaFaultPolicy::intermittent(PersonaFault::TransientIo, 2);
    let mut resilient = ResilientPersonaLoader::new(policy, 3);

    // 1. Successful load recorded in AuditLedger
    let res = resilient.load_from_dir_and_record(&mut ledger, &dir).await;
    assert!(res.is_ok());
    let entries = ledger.entries();
    assert!(!entries.is_empty());
    let allowed_event = entries.iter().find(|e| e.event_type == AuditEventType::CommandAllowed);
    assert!(allowed_event.is_some());

    // 2. Export failure recorded in AuditLedger
    let mut invalid_sample = create_sample_profile("InvalidAgent");
    invalid_sample.identity.name = "".to_string();
    let bad_export = resilient.export_to_dir_and_record(&mut ledger, &invalid_sample, &dir).await;
    assert!(bad_export.is_err());
    let updated_entries = ledger.entries();
    let blocked_event = updated_entries.iter().find(|e| e.event_type == AuditEventType::CommandBlocked);
    assert!(blocked_event.is_some());

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn test_direct_simulate_persona_fault() {
    assert!(matches!(
        simulate_persona_fault(&PersonaFault::SocketTimeout),
        Err(PersonaError::SocketTimeout)
    ));
    assert!(matches!(
        simulate_persona_fault(&PersonaFault::ConnectionReset),
        Err(PersonaError::ConnectionReset)
    ));
    assert!(matches!(
        simulate_persona_fault(&PersonaFault::TransientIo),
        Err(PersonaError::TransientIo(_))
    ));
    assert!(matches!(
        simulate_persona_fault(&PersonaFault::CorruptedProfile),
        Err(PersonaError::CorruptedProfile(_))
    ));
    assert!(matches!(
        simulate_persona_fault(&PersonaFault::InvalidIdentity),
        Err(PersonaError::InvalidIdentity(_))
    ));
    assert!(matches!(
        simulate_persona_fault(&PersonaFault::LockFailure),
        Err(PersonaError::LockFailure)
    ));
}

#[tokio::test]
async fn test_resilient_persona_alias_and_builders() {
    let mut persona: ResilientPersona = ResilientPersonaLoader::default()
        .with_max_retries(5)
        .with_policy(PersonaFaultPolicy::intermittent(PersonaFault::TransientIo, 2));

    assert_eq!(persona.max_retries(), 5);
    assert_eq!(persona.loaded_count(), 0);
    assert_eq!(persona.saved_count(), 0);
    assert_eq!(persona.recovered_count(), 0);
    assert_eq!(persona.rejected_count(), 0);

    persona.set_max_retries(4);
    assert_eq!(persona.max_retries(), 4);
    persona.set_policy(PersonaFaultPolicy::none());
    assert_eq!(persona.policy().injected_count(), 0);
}

use agent_domain::{
    ActionSeverity, AgentId, AgentIdentity, AgentMission, AgentPersonaProfile, AgentRole,
    AgentSoul, AuditLedger, AutonomyLevel, IdentityError, IdentityFault, IdentityFaultPolicy,
    ResilientIdentityManager,
};

fn create_sample_profile(id_prefix: &str, name: &str) -> AgentPersonaProfile {
    AgentPersonaProfile {
        identity: AgentIdentity {
            id: AgentId::new(id_prefix),
            name: name.to_string(),
            description: "Test agent for identity fault injection".to_string(),
            autonomy_level: AutonomyLevel::Supervised,
            created_at: chrono::Utc::now(),
        },
        role: AgentRole {
            role_code: "RESEARCHER".to_string(),
            title: "Research Agent".to_string(),
            primary_domain: "Distributed Systems".to_string(),
            capabilities: vec!["fault_injection".to_string(), "resilience_testing".to_string()],
        },
        soul: AgentSoul {
            persona: "Methodical and resilient".to_string(),
            tone_of_voice: "Precise".to_string(),
            core_values: vec!["Integrity".to_string(), "Reliability".to_string()],
            voice_profile: None,
        },
        mission: AgentMission {
            high_level_objective: "Eliminate repetitive system faults".to_string(),
            kpis: vec!["100% fault recovery".to_string()],
            constraints: vec!["No unhandled panics".to_string()],
        },
    }
}

#[test]
fn test_identity_validation_and_integrity() {
    let valid_profile = create_sample_profile("researcher", "Dr. Turing");
    assert!(valid_profile.validate().is_ok());

    // Identity with empty name must fail validation
    let mut invalid_profile = valid_profile.clone();
    invalid_profile.identity.name = "   ".to_string();
    assert!(matches!(
        invalid_profile.validate(),
        Err(IdentityError::InvalidIdentity(_))
    ));

    // Identity with empty ID must fail validation
    let mut invalid_id_profile = valid_profile.clone();
    invalid_id_profile.identity.id = AgentId::from_str("");
    assert!(matches!(
        invalid_id_profile.validate(),
        Err(IdentityError::InvalidIdentity(_))
    ));

    // Verify simulate_fault on AgentPersonaProfile
    assert!(matches!(
        valid_profile.simulate_fault(&IdentityFault::SocketTimeout),
        Err(IdentityError::SocketTimeout)
    ));
    assert!(matches!(
        valid_profile.simulate_fault(&IdentityFault::ConnectionReset),
        Err(IdentityError::ConnectionReset)
    ));
    assert!(matches!(
        valid_profile.simulate_fault(&IdentityFault::CorruptedProfile),
        Err(IdentityError::CorruptedProfile(_))
    ));
    assert!(matches!(
        valid_profile.simulate_fault(&IdentityFault::ServiceUnavailable),
        Err(IdentityError::ServiceUnavailable(_))
    ));
    assert!(matches!(
        valid_profile.simulate_fault(&IdentityFault::TransientIo),
        Err(IdentityError::TransientIo(_))
    ));
}

#[test]
fn test_identity_intermittent_socket_timeout_recovery() {
    let profile = create_sample_profile("agent", "Agent Alpha");
    let fault_policy = IdentityFaultPolicy::intermittent(IdentityFault::SocketTimeout, 2);
    let mut manager = ResilientIdentityManager::new(fault_policy, 3);

    let res = manager.resolve_identity(profile);
    assert!(res.is_ok());
    assert!(manager.recovered_count() > 0);
    assert_eq!(manager.resolved_count(), 1);
}

#[test]
fn test_identity_max_retries_exceeded_on_persistent_timeout() {
    let profile = create_sample_profile("agent", "Agent Alpha");
    let fault_policy = IdentityFaultPolicy::always(IdentityFault::SocketTimeout);
    let mut manager = ResilientIdentityManager::new(fault_policy, 2);

    let res = manager.resolve_identity(profile);
    assert_eq!(res, Err(IdentityError::SocketTimeout));
    assert_eq!(manager.resolved_count(), 0);
}

#[test]
fn test_identity_connection_reset_recovery() {
    let profile = create_sample_profile("agent", "Agent Beta");
    let fault_policy = IdentityFaultPolicy::intermittent(IdentityFault::ConnectionReset, 2);
    let mut manager = ResilientIdentityManager::new(fault_policy, 3);

    let res = manager.resolve_identity(profile);
    assert!(res.is_ok());
    assert!(manager.recovered_count() > 0);
    assert_eq!(manager.resolved_count(), 1);
}

#[test]
fn test_identity_transient_io_failure_recovery() {
    let profile = create_sample_profile("agent", "Agent Gamma");
    let fault_policy = IdentityFaultPolicy::intermittent(IdentityFault::TransientIo, 2);
    let mut manager = ResilientIdentityManager::new(fault_policy, 3);

    let res = manager.resolve_identity(profile);
    assert!(res.is_ok());
    assert!(manager.recovered_count() > 0);
    assert_eq!(manager.resolved_count(), 1);
}

#[test]
fn test_identity_corrupted_profile_rejection() {
    let profile = create_sample_profile("agent", "Agent Corrupted");
    let fault_policy = IdentityFaultPolicy::always(IdentityFault::CorruptedProfile);
    let mut manager = ResilientIdentityManager::new(fault_policy, 3);

    let res = manager.resolve_identity(profile);
    assert!(matches!(res, Err(IdentityError::CorruptedProfile(_))));
    assert_eq!(manager.resolved_count(), 0);
}

#[test]
fn test_identity_service_unavailable_recovery() {
    let profile = create_sample_profile("agent", "Agent Delta");
    let fault_policy = IdentityFaultPolicy::intermittent(IdentityFault::ServiceUnavailable, 2);
    let mut manager = ResilientIdentityManager::new(fault_policy, 3);

    let res = manager.resolve_identity(profile);
    assert!(res.is_ok());
    assert!(manager.recovered_count() > 0);
    assert_eq!(manager.resolved_count(), 1);
}

#[test]
fn test_identity_audit_ledger_chaos_integration() {
    let profile = create_sample_profile("agent", "Agent Epsilon");
    let fault_policy = IdentityFaultPolicy::intermittent(IdentityFault::SocketTimeout, 2);
    let mut manager = ResilientIdentityManager::new(fault_policy, 3);
    let mut ledger = AuditLedger::new();

    // 1. Safe identity resolution under intermittent chaos
    let res = manager.resolve_and_record(&mut ledger, profile.clone());
    assert!(res.is_ok());
    assert_eq!(manager.resolved_count(), 1);
    assert!(manager.recovered_count() > 0);

    // 2. Rejected invalid identity under chaos
    let mut invalid_profile = profile;
    invalid_profile.identity.name = "".to_string();
    let res_invalid = manager.resolve_and_record(&mut ledger, invalid_profile);
    assert!(matches!(res_invalid, Err(IdentityError::InvalidIdentity(_))));
    assert_eq!(manager.rejected_count(), 1);

    // 3. Verify audit ledger chain integrity
    assert!(ledger.verify_integrity().is_ok());
    assert_eq!(ledger.entries().len(), 2);
}

#[test]
fn test_identity_audit_ledger_persistent_failure_records_exception() {
    let profile = create_sample_profile("agent", "Agent Zeta");
    let fault_policy = IdentityFaultPolicy::always(IdentityFault::SocketTimeout);
    let mut manager = ResilientIdentityManager::new(fault_policy, 2);
    let mut ledger = AuditLedger::new();

    let res = manager.resolve_and_record(&mut ledger, profile);
    assert_eq!(res, Err(IdentityError::SocketTimeout));

    // Ledger should record the exception event
    assert!(ledger.verify_integrity().is_ok());
    assert_eq!(ledger.entries().len(), 1);
    assert_eq!(ledger.entries()[0].severity, ActionSeverity::HighImpact);
}

#[test]
fn test_high_volume_identity_chaos_resilience() {
    let fault_policy = IdentityFaultPolicy::intermittent(IdentityFault::ConnectionReset, 3);
    let mut manager = ResilientIdentityManager::new(fault_policy, 4);
    let mut ledger = AuditLedger::new();

    for i in 0..50 {
        let mut p = create_sample_profile("worker", &format!("Worker-{}", i));
        if i % 5 == 0 {
            p.identity.name = "".to_string(); // Invalid profile
        }
        let _ = manager.resolve_and_record(&mut ledger, p);
    }

    assert_eq!(ledger.entries().len(), 50);
    assert!(ledger.verify_integrity().is_ok());
    assert!(manager.recovered_count() > 0);
    assert_eq!(manager.rejected_count(), 10);
    assert_eq!(manager.resolved_count(), 40);
}

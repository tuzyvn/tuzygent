use agent_domain::audit_ledger::{
    AuditEntry, AuditEventType, AuditIntegrityError, AuditLedger, GENESIS_HASH,
};
use agent_domain::guardrail::{ActionSeverity, GuardrailPolicy};
use chrono::Utc;

#[test]
fn test_empty_audit_ledger_integrity() {
    let ledger = AuditLedger::new();
    assert!(ledger.is_empty());
    assert_eq!(ledger.len(), 0);
    assert_eq!(ledger.last_hash(), GENESIS_HASH);
    assert!(ledger.verify_integrity().is_ok());
    assert!(ledger.is_valid());
}

#[test]
fn test_audit_entry_construction_and_verification() {
    let now = Utc::now();
    let entry = AuditEntry::new(
        0,
        now,
        "SHELL_EXECUTION",
        ActionSeverity::HighImpact,
        "systemctl restart agent-server",
        GENESIS_HASH,
    )
    .with_actor("agent-admin");

    assert_eq!(entry.sequence, 0);
    assert_eq!(entry.actor, Some("agent-admin".to_string()));
    assert_eq!(entry.prev_hash, GENESIS_HASH);
    assert!(entry.verify_hash());

    let mut ledger = AuditLedger::new();
    assert!(ledger.append(entry).is_ok());
    assert_eq!(ledger.len(), 1);
    assert!(ledger.verify_integrity().is_ok());
}

#[test]
fn test_audit_ledger_record_and_hash_chaining() {
    let mut ledger = AuditLedger::new();

    let entry1 = ledger
        .record(
            "COMMAND_ALLOWED",
            ActionSeverity::Routine,
            "cargo test --workspace",
        )
        .clone();

    assert_eq!(entry1.sequence, 0);
    assert_eq!(entry1.prev_hash, GENESIS_HASH);
    assert_eq!(entry1.calculate_hash(), entry1.hash);
    assert_eq!(ledger.last_hash(), entry1.hash);

    let entry2 = ledger
        .record(
            "COMMAND_BLOCKED",
            ActionSeverity::Destructive,
            "rm -rf /var/data",
        )
        .clone();

    assert_eq!(entry2.sequence, 1);
    assert_eq!(entry2.prev_hash, entry1.hash);
    assert_eq!(entry2.calculate_hash(), entry2.hash);
    assert_eq!(ledger.last_hash(), entry2.hash);

    let entry3 = ledger
        .record_with_actor(
            "APPROVAL_REQUIRED",
            ActionSeverity::HighImpact,
            "chmod 777 /etc/passwd",
            "security-guardrail",
        )
        .clone();

    assert_eq!(entry3.sequence, 2);
    assert_eq!(entry3.actor, Some("security-guardrail".to_string()));
    assert_eq!(entry3.prev_hash, entry2.hash);
    assert_eq!(entry3.calculate_hash(), entry3.hash);

    assert_eq!(ledger.len(), 3);
    assert!(ledger.verify_integrity().is_ok());
    assert!(ledger.is_valid());
}

#[test]
fn test_guardrail_policy_audit_integration() {
    let policy = GuardrailPolicy::default_enterprise();
    let mut ledger = AuditLedger::new();

    let safe_res = policy.evaluate_and_record(&mut ledger, "cargo check");
    assert!(safe_res.is_ok());
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger.entries()[0].action, "COMMAND_ALLOWED");
    assert_eq!(ledger.entries()[0].severity, ActionSeverity::Routine);

    let danger_res = policy.evaluate_and_record(&mut ledger, "rm -rf /");
    assert!(danger_res.is_err());
    assert_eq!(ledger.len(), 2);
    assert_eq!(ledger.entries()[1].action, "COMMAND_BLOCKED");
    assert_eq!(ledger.entries()[1].severity, ActionSeverity::Destructive);

    assert!(ledger.verify_integrity().is_ok());

    // Also verify check_command_safety_with_audit alias
    let danger_res2 = policy.check_command_safety_with_audit(&mut ledger, "DROP TABLE users");
    assert!(danger_res2.is_err());
    assert_eq!(ledger.len(), 3);
    assert!(ledger.verify_integrity().is_ok());

    // Verify record_guardrail_check method on ledger
    let check_res = ledger.record_guardrail_check(&policy, "truncate users");
    assert!(check_res.is_err());
    assert_eq!(ledger.len(), 4);
    assert!(ledger.verify_integrity().is_ok());
}

#[test]
fn test_tamper_detection_modified_payload() {
    let mut ledger = AuditLedger::new();
    ledger.record("ACTION_A", ActionSeverity::Routine, "Normal action");
    ledger.record("ACTION_B", ActionSeverity::Routine, "Another action");

    assert!(ledger.verify_integrity().is_ok());

    // Tamper with entry 0's details without updating its hash
    let mut tampered_entries = ledger.entries().to_vec();
    tampered_entries[0].details = "Tampered malicious payload".to_string();

    let tampered_ledger = AuditLedger::from_entries(tampered_entries);
    match tampered_ledger.verify_integrity() {
        Err(AuditIntegrityError::TamperedHash { sequence, .. }) => {
            assert_eq!(sequence, 0);
        }
        other => panic!("Expected TamperedHash error, got: {:?}", other),
    }
    assert!(!tampered_ledger.is_valid());
}

#[test]
fn test_tamper_detection_broken_chain() {
    let mut ledger = AuditLedger::new();
    ledger.record("ACTION_1", ActionSeverity::Routine, "Event 1");
    ledger.record("ACTION_2", ActionSeverity::Routine, "Event 2");
    ledger.record("ACTION_3", ActionSeverity::Routine, "Event 3");

    // Tamper with entry 1: recompute its hash so entry 1's own hash matches its payload,
    // but entry 2's prev_hash still points to the old hash.
    let mut tampered_entries = ledger.entries().to_vec();
    tampered_entries[1].details = "Modified event 2".to_string();
    tampered_entries[1].hash = tampered_entries[1].calculate_hash();

    let tampered_ledger = AuditLedger::from_entries(tampered_entries);
    match tampered_ledger.verify_integrity() {
        Err(AuditIntegrityError::InvalidPrevHash { sequence, .. }) => {
            assert_eq!(sequence, 2);
        }
        other => panic!(
            "Expected InvalidPrevHash error at sequence 2, got: {:?}",
            other
        ),
    }
}

#[test]
fn test_tamper_detection_invalid_sequence() {
    let mut ledger = AuditLedger::new();
    ledger.record("A", ActionSeverity::Routine, "detail A");
    ledger.record("B", ActionSeverity::Routine, "detail B");

    let mut tampered_entries = ledger.entries().to_vec();
    tampered_entries[1].sequence = 99; // invalid sequence

    let tampered_ledger = AuditLedger::from_entries(tampered_entries);
    match tampered_ledger.verify_integrity() {
        Err(AuditIntegrityError::InvalidSequence {
            index,
            expected,
            found,
        }) => {
            assert_eq!(index, 1);
            assert_eq!(expected, 1);
            assert_eq!(found, 99);
        }
        other => panic!("Expected InvalidSequence error, got: {:?}", other),
    }
}

#[test]
fn test_guardrail_action_lifecycle_allowed_blocked_exception_and_tamper_detection() {
    let policy = GuardrailPolicy::default_enterprise();
    let mut ledger = AuditLedger::new();

    // 1. Cho phép (Allowed command)
    let safe_cmd = "cargo build --release";
    let safe_eval = policy.evaluate_and_record(&mut ledger, safe_cmd);
    assert!(safe_eval.is_ok());

    // 2. Chặn lệnh nguy hiểm (Blocked dangerous command)
    let dangerous_cmd = "rm -rf /var/log/audit";
    let blocked_eval = policy.evaluate_and_record(&mut ledger, dangerous_cmd);
    assert!(blocked_eval.is_err());

    // 3. Ngoại lệ trong xử lý / thực thi lệnh (Exception)
    let failing_cmd = "curl -f https://internal.service/health";
    policy.record_exception(
        &mut ledger,
        failing_cmd,
        "Network connection reset by peer (ConnectionReset)",
    );

    // 4. Chặn lệnh nguy hiểm khác (SQL Drop)
    let sql_injection = "DROP TABLE users; --";
    let blocked_eval2 = policy.evaluate_and_record(&mut ledger, sql_injection);
    assert!(blocked_eval2.is_err());

    // 5. Ngoại lệ cấp hệ thống khác (Ledger exception recording)
    ledger.record_exception(
        "SANDBOX_CONTAINER_FAULT",
        "OOMKilled: Process exceeded memory quota 512MB",
    );

    // Kiểm tra cấu trúc bản ghi và xác nhận chuỗi ban đầu hợp lệ
    assert_eq!(ledger.len(), 5);
    assert_eq!(ledger.entries()[0].action, "COMMAND_ALLOWED");
    assert_eq!(ledger.entries()[0].severity, ActionSeverity::Routine);
    assert_eq!(ledger.entries()[1].action, "COMMAND_BLOCKED");
    assert_eq!(ledger.entries()[1].severity, ActionSeverity::Destructive);
    assert_eq!(ledger.entries()[2].action, "COMMAND_EXCEPTION");
    assert_eq!(ledger.entries()[2].severity, ActionSeverity::HighImpact);
    assert_eq!(ledger.entries()[3].action, "COMMAND_BLOCKED");
    assert_eq!(ledger.entries()[3].severity, ActionSeverity::Destructive);
    assert_eq!(ledger.entries()[4].action, "SANDBOX_CONTAINER_FAULT");
    assert_eq!(ledger.entries()[4].severity, ActionSeverity::HighImpact);

    // Kiểm tra verify_integrity() trả về Ok với chuỗi hợp lệ
    assert!(ledger.verify_integrity().is_ok());
    assert!(ledger.is_valid());

    // Kịch bản can thiệp 1: Kẻ tấn công sửa đổi hành vi chặn lệnh thành cho phép (Tampered Action)
    {
        let mut tampered = ledger.entries().to_vec();
        tampered[1].action = "COMMAND_ALLOWED".to_string();
        let tampered_ledger = AuditLedger::from_entries(tampered);
        match tampered_ledger.verify_integrity() {
            Err(AuditIntegrityError::TamperedHash { sequence, .. }) => {
                assert_eq!(sequence, 1);
            }
            res => panic!("Expected TamperedHash at sequence 1, got {:?}", res),
        }
        assert!(!tampered_ledger.is_valid());
    }

    // Kịch bản can thiệp 2: Kẻ tấn công tẩy trắng thông tin ngoại lệ (Tampered Details)
    {
        let mut tampered = ledger.entries().to_vec();
        tampered[2].details = "Status 200 OK".to_string();
        let tampered_ledger = AuditLedger::from_entries(tampered);
        match tampered_ledger.verify_integrity() {
            Err(AuditIntegrityError::TamperedHash { sequence, .. }) => {
                assert_eq!(sequence, 2);
            }
            res => panic!("Expected TamperedHash at sequence 2, got {:?}", res),
        }
        assert!(!tampered_ledger.is_valid());
    }

    // Kịch bản can thiệp 3: Kẻ tấn công hạ mức nghiêm trọng của sự kiện (Tampered Severity)
    {
        let mut tampered = ledger.entries().to_vec();
        tampered[3].severity = ActionSeverity::Routine;
        let tampered_ledger = AuditLedger::from_entries(tampered);
        match tampered_ledger.verify_integrity() {
            Err(AuditIntegrityError::TamperedHash { sequence, .. }) => {
                assert_eq!(sequence, 3);
            }
            res => panic!("Expected TamperedHash at sequence 3, got {:?}", res),
        }
        assert!(!tampered_ledger.is_valid());
    }

    // Kịch bản can thiệp 4: Kẻ tấn công tính lại hash của bản ghi bị sửa đổi nhưng làm đứt gãy chuỗi băm (InvalidPrevHash)
    {
        let mut tampered = ledger.entries().to_vec();
        tampered[1].details = "Tampered details with recomputed entry hash".to_string();
        tampered[1].hash = tampered[1].calculate_hash();
        // tampered[2].prev_hash vẫn trỏ tới hash ban đầu của tampered[1]
        let tampered_ledger = AuditLedger::from_entries(tampered);
        match tampered_ledger.verify_integrity() {
            Err(AuditIntegrityError::InvalidPrevHash { sequence, .. }) => {
                assert_eq!(sequence, 2);
            }
            res => panic!("Expected InvalidPrevHash at sequence 2, got {:?}", res),
        }
        assert!(!tampered_ledger.is_valid());
    }

    // Kịch bản can thiệp 5: Kẻ tấn công xoá bỏ bản ghi vi phạm để xoá dấu vết (InvalidSequence)
    {
        let mut tampered = ledger.entries().to_vec();
        tampered.remove(1); // Xoá bỏ lệnh vi phạm nguy hiểm ở index 1
        let tampered_ledger = AuditLedger::from_entries(tampered);
        match tampered_ledger.verify_integrity() {
            Err(AuditIntegrityError::InvalidSequence {
                index,
                expected,
                found,
            }) => {
                assert_eq!(index, 1);
                assert_eq!(expected, 1);
                assert_eq!(found, 2);
            }
            res => panic!("Expected InvalidSequence at index 1, got {:?}", res),
        }
        assert!(!tampered_ledger.is_valid());
    }
}

#[test]
fn test_audit_event_type_classification_and_serde() {
    assert!(AuditEventType::CommandAllowed.is_allowed());
    assert!(!AuditEventType::CommandAllowed.is_blocked());
    assert!(!AuditEventType::CommandAllowed.is_exception());

    assert!(AuditEventType::CommandBlocked.is_blocked());
    assert!(AuditEventType::PolicyViolation.is_blocked());
    assert!(!AuditEventType::CommandBlocked.is_allowed());

    assert!(AuditEventType::CommandException.is_exception());
    assert!(!AuditEventType::CommandException.is_allowed());

    let custom = AuditEventType::Custom("FILE_ENCRYPTED".to_string());
    assert_eq!(custom.as_str(), "FILE_ENCRYPTED");
    assert_eq!(custom.to_string(), "FILE_ENCRYPTED");

    // Serde roundtrip
    let allowed_json = serde_json::to_string(&AuditEventType::CommandAllowed).unwrap();
    assert_eq!(allowed_json, "\"COMMAND_ALLOWED\"");
    let deserialized: AuditEventType = serde_json::from_str(&allowed_json).unwrap();
    assert_eq!(deserialized, AuditEventType::CommandAllowed);

    let custom_json = serde_json::to_string(&custom).unwrap();
    assert_eq!(custom_json, "\"FILE_ENCRYPTED\"");
    let deserialized_custom: AuditEventType = serde_json::from_str(&custom_json).unwrap();
    assert_eq!(deserialized_custom, custom);
}

#[test]
fn test_audit_ledger_record_event_and_clean_construction() {
    let mut ledger = AuditLedger::new();

    // Use record_event directly with AuditEventType
    let entry = ledger
        .record_event(
            AuditEventType::CommandAllowed,
            ActionSeverity::Routine,
            "cargo test --workspace",
        )
        .clone();

    assert_eq!(entry.sequence, 0);
    assert_eq!(entry.event_type, AuditEventType::CommandAllowed);
    assert_eq!(entry.action, "COMMAND_ALLOWED");
    assert_eq!(entry.prev_hash, GENESIS_HASH);
    assert!(entry.verify_hash());

    let blocked_entry = ledger
        .record_event(
            AuditEventType::CommandBlocked,
            ActionSeverity::Destructive,
            "rm -rf /root",
        )
        .clone();

    assert_eq!(blocked_entry.sequence, 1);
    assert_eq!(blocked_entry.event_type, AuditEventType::CommandBlocked);
    assert_eq!(blocked_entry.prev_hash, entry.hash);
    assert!(blocked_entry.verify_hash());

    assert!(ledger.verify_integrity().is_ok());
    assert_eq!(ledger.len(), 2);
}

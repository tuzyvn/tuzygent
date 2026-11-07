use agent_domain::{
    ActionSeverity, AuditIntegrityError, AuditLedger, GuardrailPolicy, GENESIS_HASH,
};

#[test]
fn test_runtime_guardrail_audit_lifecycle_integration() {
    let policy = GuardrailPolicy::default_enterprise();
    let mut ledger = AuditLedger::new();

    assert_eq!(ledger.last_hash(), GENESIS_HASH);

    // 1. Cho phép: Thao tác an toàn được phê duyệt và ghi nhận vào sổ kiểm toán
    let safe_cmd = "git status --porcelain";
    let safe_res = policy.evaluate_and_record(&mut ledger, safe_cmd);
    assert!(safe_res.is_ok(), "Lệnh an toàn phải được chấp thuận");

    // 2. Chặn lệnh nguy hiểm: Phát hiện và chặn lệnh xoá phá huỷ
    let destructive_cmd = "rm -rf /var/lib/tuzygent/data";
    let blocked_res = policy.evaluate_and_record(&mut ledger, destructive_cmd);
    assert!(blocked_res.is_err(), "Lệnh phá huỷ phải bị chặn");

    // 3. Ngoại lệ: Quá trình thực thi gặp sự cố ngoại lệ (Timeout/Crash/Sandbox fault)
    let faulty_cmd = "python3 -c 'import os; os.abort()'";
    policy.record_exception(
        &mut ledger,
        faulty_cmd,
        "RuntimeSandboxError: Process killed abruptly with SIGABRT",
    );

    // 4. Chặn lệnh nguy hiểm thứ hai: Lệnh sửa quyền truy cập nguy hiểm
    let perm_cmd = "chmod 777 /etc/shadow";
    let blocked_perm = policy.evaluate_and_record(&mut ledger, perm_cmd);
    assert!(
        blocked_perm.is_err(),
        "Lệnh thay đổi quyền 777 phải bị chặn"
    );

    // 5. Ngoại lệ cấp hệ thống
    ledger.record_exception(
        "WORKFLOW_STEP_TIMEOUT",
        "Subtask step exceeded maximum allocated budget (120s)",
    );

    // Xác nhận độ dài và thứ tự tuần tự
    assert_eq!(ledger.len(), 5);
    assert_eq!(ledger.entries()[0].severity, ActionSeverity::Routine);
    assert_eq!(ledger.entries()[1].severity, ActionSeverity::Destructive);
    assert_eq!(ledger.entries()[2].severity, ActionSeverity::HighImpact);
    assert_eq!(ledger.entries()[3].severity, ActionSeverity::Destructive);
    assert_eq!(ledger.entries()[4].severity, ActionSeverity::HighImpact);

    for (i, entry) in ledger.entries().iter().enumerate() {
        assert_eq!(entry.sequence, i as u64);
        assert!(
            entry.verify_hash(),
            "Hash của mỗi bản ghi phải hợp lệ với payload"
        );
    }

    // Kiểm tra verify_integrity() trả về Ok với chuỗi hợp lệ
    let verify_result = ledger.verify_integrity();
    assert!(
        verify_result.is_ok(),
        "Chuỗi kiểm toán nguyên bản phải vượt qua kiểm tra toàn vẹn"
    );
    assert!(ledger.is_valid());

    // Kiểm tra liên kết băm (hash chain)
    assert_eq!(ledger.entries()[0].prev_hash, GENESIS_HASH);
    assert_eq!(ledger.entries()[1].prev_hash, ledger.entries()[0].hash);
    assert_eq!(ledger.entries()[2].prev_hash, ledger.entries()[1].hash);
    assert_eq!(ledger.entries()[3].prev_hash, ledger.entries()[2].hash);
    assert_eq!(ledger.entries()[4].prev_hash, ledger.entries()[3].hash);
    assert_eq!(ledger.last_hash(), ledger.entries()[4].hash);
}

#[test]
fn test_runtime_guardrail_audit_tamper_detection() {
    let policy = GuardrailPolicy::default_enterprise();
    let mut ledger = AuditLedger::new();

    // Thiết lập chuỗi các hành vi guardrail: cho phép, chặn nguy hiểm, ngoại lệ
    policy
        .evaluate_and_record(&mut ledger, "cargo check --workspace")
        .expect("safe check");
    let _ = policy.evaluate_and_record(&mut ledger, "rm -rf /");
    policy.record_exception(&mut ledger, "bad_alloc_cmd", "MemoryAllocationFailed");
    let _ = policy.evaluate_and_record(&mut ledger, "DROP TABLE sensitive_data");

    assert!(ledger.verify_integrity().is_ok());

    // 1. Can thiệp trái phép: Sửa đổi trường action của bản ghi bị chặn
    {
        let mut tampered_entries = ledger.entries().to_vec();
        tampered_entries[1].action = "COMMAND_ALLOWED".to_string();
        let tampered_ledger = AuditLedger::from_entries(tampered_entries);
        let result = tampered_ledger.verify_integrity();
        match result {
            Err(AuditIntegrityError::TamperedHash { sequence, .. }) => {
                assert_eq!(sequence, 1, "Phải phát hiện sửa đổi hash tại sequence 1");
            }
            other => panic!("Kỳ vọng lỗi TamperedHash, nhận được: {:?}", other),
        }
        assert!(!tampered_ledger.is_valid());
    }

    // 2. Can thiệp trái phép: Sửa đổi details của bản ghi ngoại lệ
    {
        let mut tampered_entries = ledger.entries().to_vec();
        tampered_entries[2].details = "Normal execution completed".to_string();
        let tampered_ledger = AuditLedger::from_entries(tampered_entries);
        let result = tampered_ledger.verify_integrity();
        match result {
            Err(AuditIntegrityError::TamperedHash { sequence, .. }) => {
                assert_eq!(sequence, 2, "Phải phát hiện sửa đổi hash tại sequence 2");
            }
            other => panic!("Kỳ vọng lỗi TamperedHash, nhận được: {:?}", other),
        }
        assert!(!tampered_ledger.is_valid());
    }

    // 3. Can thiệp trái phép: Kẻ tấn công băm lại bản ghi nhưng chuỗi liên kết prev_hash bị đứt
    {
        let mut tampered_entries = ledger.entries().to_vec();
        tampered_entries[1].details = "Command sanitized and approved".to_string();
        tampered_entries[1].hash = tampered_entries[1].calculate_hash();
        let tampered_ledger = AuditLedger::from_entries(tampered_entries);
        let result = tampered_ledger.verify_integrity();
        match result {
            Err(AuditIntegrityError::InvalidPrevHash { sequence, .. }) => {
                assert_eq!(
                    sequence, 2,
                    "Bản ghi kế tiếp phải phát hiện prev_hash không khớp"
                );
            }
            other => panic!("Kỳ vọng lỗi InvalidPrevHash, nhận được: {:?}", other),
        }
        assert!(!tampered_ledger.is_valid());
    }

    // 4. Can thiệp trái phép: Kẻ tấn công xoá bỏ một bản ghi vi phạm trong chuỗi
    {
        let mut tampered_entries = ledger.entries().to_vec();
        tampered_entries.remove(1); // Xoá bỏ entry sequence 1
        let tampered_ledger = AuditLedger::from_entries(tampered_entries);
        let result = tampered_ledger.verify_integrity();
        match result {
            Err(AuditIntegrityError::InvalidSequence {
                index,
                expected,
                found,
            }) => {
                assert_eq!(index, 1);
                assert_eq!(expected, 1);
                assert_eq!(found, 2);
            }
            other => panic!("Kỳ vọng lỗi InvalidSequence, nhận được: {:?}", other),
        }
        assert!(!tampered_ledger.is_valid());
    }
}

use agent_domain::{
    holds_credential, is_secret_key_name, mask_secret, redact_json_value, redact_secrets_in_text,
};
use serde_json::json;

#[test]
fn mask_secret_is_idempotent_on_marker() {
    let masked = mask_secret("super-secret-value");
    assert_eq!(masked, "«redacted 18 chars»");
    assert_eq!(mask_secret(&masked), masked);
}

#[test]
fn is_secret_key_name_avoids_hotkey_and_keyboard_false_positives() {
    assert!(is_secret_key_name("OPENAI_API_KEY"));
    assert!(is_secret_key_name("auth_token"));
    assert!(is_secret_key_name("db_password"));
    assert!(is_secret_key_name("encryption_keys"));
    assert!(is_secret_key_name("OMB_GATE_UPSTREAM"));

    assert!(!is_secret_key_name("hotkey"));
    assert!(!is_secret_key_name("keyboard"));
    assert!(!is_secret_key_name("user_name"));
}

#[test]
fn holds_credential_detects_provider_tokens_and_pem_only() {
    let openai = format!("{}{}", "sk-proj-", "abcdefghijklmnop123456");
    let aws = format!("{}{}", "AKIA", "IOSFODNN7EXAMPLE");
    let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIBOgIBAAJB\n-----END RSA PRIVATE KEY-----";

    assert!(holds_credential(&openai));
    assert!(holds_credential(&aws));
    assert!(holds_credential(pem));
    assert!(!holds_credential("password: leave blank unless using a vault"));
    assert!(!holds_credential("const key = 'placeholder';"));
}

#[test]
fn redact_secrets_in_text_masks_tokens_headers_assignments_flags_urls_and_is_idempotent() {
    let openai = format!("{}{}", "sk-ant-", "secret1234567890abcdef");
    let bearer = "Bearer eyJhbGciOiJIUzI1NiJ9.payload123.signature12";
    let basic = "Authorization: Basic dXNlcjpwYXNzd29yZA==";
    let url = "postgres://alice:s3cr3tp4ss@db.internal:5432/app";
    let flag = "--api-key my-flag-secret-99";
    let assign = "CUSTOM_KEY=val1 password: not_masked_short api_token=abcdef1234567890";
    let safe_utf8 = "Khóa tiếng Việt không bị lỗi cắt ký tự và api_key=validSecret12345";

    let combined = format!(
        "{openai}\n{bearer}\n{basic}\n{url}\n{flag}\n{assign}\n{safe_utf8}\nhotkey=Ctrl+C"
    );

    let first_pass = redact_secrets_in_text(&combined);
    assert!(!first_pass.contains("secret1234567890abcdef"));
    assert!(!first_pass.contains("dXNlcjpwYXNzd29yZA=="));
    assert!(!first_pass.contains("s3cr3tp4ss"));
    assert!(!first_pass.contains("my-flag-secret-99"));
    assert!(!first_pass.contains("abcdef1234567890"));
    assert!(!first_pass.contains("validSecret12345"));
    assert!(first_pass.contains("hotkey=Ctrl+C"));
    assert!(first_pass.contains("«redacted "));

    let second_pass = redact_secrets_in_text(&first_pass);
    assert_eq!(second_pass, first_pass);
}

#[test]
fn redact_json_value_masks_secret_keys_and_name_value_pairs() {
    let payload = json!({
        "api_key": "short",
        "nested": {
            "Authorization": "Bearer abcdefghijklmnopqrstuvwxyz",
            "title": "deploy ghp_1234567890abcdefghij1234"
        },
        "env": [
            { "name": "MY_SECRET_TOKEN", "value": "raw-token-val" },
            { "name": "PUBLIC_LABEL", "value": "hello world" }
        ]
    });

    let redacted = redact_json_value(&payload);
    assert_eq!(redacted["api_key"], "«redacted 5 chars»");
    assert_eq!(redacted["nested"]["Authorization"], "«redacted 33 chars»");
    assert!(!redacted["nested"]["title"]
        .as_str()
        .unwrap()
        .contains("ghp_1234567890"));
    assert_eq!(redacted["env"][0]["value"], "«redacted 13 chars»");
    assert_eq!(redacted["env"][1]["value"], "hello world");
    assert_eq!(redact_json_value(&redacted), redacted);
}

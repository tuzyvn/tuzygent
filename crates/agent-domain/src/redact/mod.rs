mod patterns;
mod prefixes;

use serde_json::{Map, Value};

struct RedactOps;

trait PatternScanners {
    fn left_word_boundary(bytes: &[u8], idx: usize) -> bool;
    fn right_word_boundary(bytes: &[u8], idx: usize) -> bool;
    fn scan_while(bytes: &[u8], i: usize, pred: fn(u8) -> bool) -> usize;
    fn find_pem_span(text: &str, from: usize) -> Option<(usize, usize, usize, usize)>;
    fn redact_pem_blocks(text: &str) -> String;
    fn redact_auth_headers(text: &str) -> String;
    fn redact_key_assignments(text: &str) -> String;
    fn redact_secret_flags(text: &str) -> String;
    fn redact_url_userinfo(text: &str) -> String;
}

trait PrefixScanners {
    fn match_known_credential_prefix(text: &str, idx: usize) -> Option<usize>;
    fn redact_known_prefixes(text: &str) -> String;
}

const SECRET_KEY_PARTS: &[&str] = &[
    "token",
    "secret",
    "password",
    "passwd",
    "apikey",
    "api_key",
    "authorization",
    "auth_token",
    "access_key",
    "private_key",
];

/// Mask a secret using an idempotent character-count placeholder so repeated passes never mutate it.
pub fn mask_secret(value: &str) -> String {
    if is_redaction_marker(value) {
        return value.to_string();
    }
    format!("«redacted {} chars»", value.chars().count())
}

fn is_redaction_marker(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("«redacted ") else {
        return false;
    };
    let Some(digits) = rest.strip_suffix(" chars»") else {
        return false;
    };
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

/// Return true if a configuration or JSON field name looks like a credential rather than ordinary prose or `hotkey`/`keyboard`.
pub fn is_secret_key_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower == "omb_gate_upstream"
        || lower == "omb_remote_mcp_server"
        || lower.starts_with("omb_remote_mcp_config_")
    {
        return true;
    }
    if SECRET_KEY_PARTS.iter().any(|part| lower.contains(part)) {
        return true;
    }
    let base = lower.strip_suffix('s').unwrap_or(&lower);
    if let Some(prefix) = base.strip_suffix("key") {
        return prefix.is_empty()
            || prefix.ends_with('_')
            || prefix.ends_with('-')
            || prefix.ends_with('.');
    }
    false
}

/// Return true only for high-precision credential shapes (provider prefixes or PEM private keys).
pub fn holds_credential(text: &str) -> bool {
    if RedactOps::find_pem_span(text, 0).is_some() {
        return true;
    }
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if text.is_char_boundary(i) && RedactOps::left_word_boundary(bytes, i) {
            if let Some(end) = RedactOps::match_known_credential_prefix(text, i) {
                if end > i {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

/// Scrub high-precision secret shapes in free text while preserving surrounding structure.
pub fn redact_secrets_in_text(text: &str) -> String {
    if text.len() < 8 {
        return text.to_string();
    }
    let step1 = RedactOps::redact_pem_blocks(text);
    let step2 = RedactOps::redact_known_prefixes(&step1);
    let step3 = RedactOps::redact_auth_headers(&step2);
    let step4 = RedactOps::redact_key_assignments(&step3);
    let step5 = RedactOps::redact_secret_flags(&step4);
    RedactOps::redact_url_userinfo(&step5)
}

/// Recursively scrub JSON objects, arrays (`name`/`value` pairs), and string values.
pub fn redact_json_value(value: &Value) -> Value {
    redact_json_inner(value, 0)
}

fn redact_json_inner(value: &Value, depth: usize) -> Value {
    match value {
        Value::String(s) => Value::String(redact_secrets_in_text(s)),
        Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
        Value::Array(items) if depth <= 12 => Value::Array(
            items
                .iter()
                .map(|item| redact_json_array_item(item, depth + 1))
                .collect(),
        ),
        Value::Object(map) if depth <= 12 => {
            let mut out = Map::with_capacity(map.len());
            for (k, v) in map {
                let next = match v {
                    Value::String(s) if is_secret_key_name(k) => Value::String(mask_secret(s)),
                    _ => redact_json_inner(v, depth + 1),
                };
                out.insert(k.clone(), next);
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

fn redact_json_array_item(item: &Value, depth: usize) -> Value {
    if let Value::Object(map) = item {
        if let (Some(Value::String(name)), Some(Value::String(val))) =
            (map.get("name"), map.get("value"))
        {
            let mut out = map.clone();
            let masked = if is_secret_key_name(name) {
                mask_secret(val)
            } else {
                redact_secrets_in_text(val)
            };
            out.insert("value".to_string(), Value::String(masked));
            return Value::Object(out);
        }
    }
    redact_json_inner(item, depth)
}

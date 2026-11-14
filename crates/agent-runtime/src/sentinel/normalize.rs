use agent_domain::redact_secrets_in_text;

const MAX_SIGNATURE_TEXT: usize = 240;

const SHELL_ERROR_CLASSES: &[(&[&str], &str)] = &[
    (&["timed out", "timed-out", "time out", "timeout"], "timed out"),
    (
        &[
            "command not found",
            "is not recognized as an internal or external command",
        ],
        "command not found",
    ),
    (
        &["syntax error", "unexpected token", "unexpected eof", "parse error"],
        "syntax error",
    ),
    (
        &["no such file or directory", "enoent", "cannot find the path"],
        "no such file",
    ),
    (&["permission denied", "eacces"], "permission denied"),
    (&["operation not permitted", "eperm"], "operation not permitted"),
    (
        &[
            "connection refused",
            "could not resolve host",
            "network is unreachable",
            "econnrefused",
            "enotfound",
        ],
        "network error",
    ),
    (&["out of memory", "cannot allocate memory", "enomem"], "out of memory"),
    (
        &["killed", "sigkill", "sigterm", "terminated by signal"],
        "killed",
    ),
    (
        &[
            "user rejected",
            "user denied",
            "user doesn't want",
            "was denied",
            "not allowed",
        ],
        "denied",
    ),
];

/// Normalize volatile paths, IDs, URLs, emails, quoted values, and numbers for error clustering.
pub fn normalize_signature_text(text: &str) -> String {
    let redacted = redact_secrets_in_text(text);
    let q_replaced = replace_quoted_spans(&redacted);
    let tokens: Vec<String> = q_replaced
        .split_whitespace()
        .map(normalize_token)
        .collect();
    let collapsed = tokens.join(" ");
    let trimmed = collapsed.trim();
    trimmed.chars().take(MAX_SIGNATURE_TEXT).collect()
}

/// Classify a shell failure into an optional exit code (omitted on timeouts) and canonical error class.
pub fn classify_shell_error(text: &str) -> (Option<i32>, &'static str) {
    let lower = text.to_ascii_lowercase();
    let error_class = SHELL_ERROR_CLASSES
        .iter()
        .find_map(|(patterns, label)| {
            if patterns.iter().any(|p| contains_phrase(&lower, p)) {
                Some(*label)
            } else {
                None
            }
        })
        .unwrap_or("failed");

    let exit_code = if error_class == "timed out" {
        None
    } else {
        extract_exit_code(&lower)
    };
    (exit_code, error_class)
}

fn contains_phrase(haystack: &str, phrase: &str) -> bool {
    if !phrase.is_ascii() {
        return haystack.contains(phrase);
    }
    let bytes = haystack.as_bytes();
    let mut from = 0;
    while let Some(rel) = haystack.get(from..).and_then(|s| s.find(phrase)) {
        let start = from + rel;
        let end = start + phrase.len();
        let left_ok = start == 0
            || !bytes
                .get(start - 1)
                .map(|b| b.is_ascii_alphanumeric() || *b == b'_')
                .unwrap_or(false);
        let right_ok = !bytes
            .get(end)
            .map(|b| b.is_ascii_alphanumeric() || *b == b'_')
            .unwrap_or(false);
        if left_ok && right_ok {
            return true;
        }
        from = start + 1;
    }
    false
}

fn extract_exit_code(lower: &str) -> Option<i32> {
    let bytes = lower.as_bytes();
    let mut from = 0;
    while let Some(rel) = lower.get(from..).and_then(|s| s.find("exit")) {
        let idx = from + rel;
        let left_ok = idx == 0
            || !bytes
                .get(idx - 1)
                .map(|b| b.is_ascii_alphanumeric())
                .unwrap_or(false);
        if left_ok {
            let mut rest = lower.get(idx + 4..).unwrap_or_default().trim_start();
            for prefix in ["ed with", "code", "status"] {
                if let Some(after) = rest.strip_prefix(prefix) {
                    rest = after.trim_start();
                }
            }
            rest = rest.trim_start_matches([':', '=']).trim_start();
            let mut len = 0;
            for (pos, b) in rest.bytes().enumerate() {
                if (pos == 0 && b == b'-') || b.is_ascii_digit() {
                    len = pos + 1;
                } else {
                    break;
                }
            }
            if let Some(code_str) = rest.get(..len) {
                if let Ok(code) = code_str.parse::<i32>() {
                    return Some(code);
                }
            }
        }
        from = idx + 4;
    }
    None
}

fn replace_quoted_spans(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if matches!(ch, '"' | '`')
            || (ch == '\''
                && !chars
                    .get(i.wrapping_sub(1))
                    .map(|c| c.is_alphanumeric() || *c == '_')
                    .unwrap_or(false))
        {
            if let Some(close) = find_closing_quote(&chars, i, ch) {
                out.push_str("<q>");
                i = close + 1;
                continue;
            }
        }
        out.push(ch);
        i += 1;
    }
    out
}

fn find_closing_quote(chars: &[char], open_idx: usize, quote: char) -> Option<usize> {
    let mut j = open_idx + 1;
    while j < chars.len() {
        let c = chars[j];
        if c == '\n' {
            return None;
        }
        if c == quote {
            if quote == '\''
                && chars
                    .get(j + 1)
                    .map(|next| next.is_alphanumeric() || *next == '_')
                    .unwrap_or(false)
            {
                return None;
            }
            return Some(j);
        }
        j += 1;
    }
    None
}

fn normalize_token(tok: &str) -> String {
    if tok == "<q>" {
        return "<q>".to_string();
    }
    let (lead, core, trail) = split_wrapping_punct(tok);
    if core.is_empty() {
        return tok.to_string();
    }
    let replaced = if is_url(core) {
        "<url>".to_string()
    } else if is_email(core) {
        "<email>".to_string()
    } else if is_path(core) {
        "<path>".to_string()
    } else if is_uuid(core) || is_hex_or_prefixed_id(core) {
        "<id>".to_string()
    } else if is_numeric_segment(core) {
        "<n>".to_string()
    } else {
        core.to_string()
    };
    format!("{lead}{replaced}{trail}")
}

fn split_wrapping_punct(tok: &str) -> (&str, &str, &str) {
    let mut start = 0;
    for (idx, ch) in tok.char_indices() {
        if matches!(ch, '(' | '[' | '{' | ',' | ';') {
            start = idx + ch.len_utf8();
        } else {
            break;
        }
    }
    let rest = tok.get(start..).unwrap_or_default();
    let mut end = rest.len();
    for (idx, ch) in rest.char_indices().rev() {
        if matches!(ch, ')' | ']' | '}' | ',' | ';' | '.' | ':') && !rest.ends_with('/') {
            end = idx;
        } else {
            break;
        }
    }
    let core = rest.get(..end).unwrap_or_default();
    let trail = rest.get(end..).unwrap_or_default();
    (tok.get(..start).unwrap_or_default(), core, trail)
}

fn is_url(s: &str) -> bool {
    let Some(pos) = s.find("://") else {
        return false;
    };
    let scheme = s.get(..pos).unwrap_or_default();
    !scheme.is_empty()
        && scheme
            .bytes()
            .next()
            .map(|b| b.is_ascii_alphabetic())
            .unwrap_or(false)
        && scheme
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'.' | b'-'))
}

fn is_email(s: &str) -> bool {
    let Some((user, domain)) = s.split_once('@') else {
        return false;
    };
    !user.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && user
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'+' | b'-'))
}

fn is_path(s: &str) -> bool {
    let bytes = s.as_bytes();
    let is_windows = bytes.len() >= 3
        && bytes.first().map(|b| b.is_ascii_alphabetic()).unwrap_or(false)
        && bytes.get(1) == Some(&b':')
        && bytes.get(2) == Some(&b'\\');
    if is_windows {
        return true;
    }
    (s.starts_with('/') || s.starts_with("~/") || s.starts_with("./") || s.starts_with("../"))
        && s.len() > 1
}

fn is_uuid(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 5 {
        return false;
    }
    let expected = [8usize, 4, 4, 4, 12];
    parts
        .iter()
        .zip(expected)
        .all(|(p, len)| p.len() == len && p.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn is_hex_or_prefixed_id(s: &str) -> bool {
    if s.len() >= 8 && s.bytes().all(|b| b.is_ascii_hexdigit()) {
        let has_digit = s.bytes().any(|b| b.is_ascii_digit());
        let has_hex_letter = s.bytes().any(|b| b.is_ascii_alphabetic());
        return has_digit && has_hex_letter;
    }
    if let Some(sep_idx) = s.find(['_', '-']) {
        let prefix = s.get(..sep_idx).unwrap_or_default();
        let rest = s.get(sep_idx + 1..).unwrap_or_default();
        return (1..=6).contains(&prefix.len())
            && prefix.bytes().all(|b| b.is_ascii_alphabetic())
            && rest.len() >= 4
            && rest.bytes().any(|b| b.is_ascii_digit())
            && rest
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    }
    false
}

fn is_numeric_segment(s: &str) -> bool {
    let mut has_digit = false;
    for b in s.bytes() {
        if b.is_ascii_digit() {
            has_digit = true;
        } else if !matches!(b, b'.' | b',' | b':') {
            return false;
        }
    }
    has_digit && !s.starts_with('.') && !s.ends_with('.')
}

use super::{is_secret_key_name, mask_secret, PatternScanners, RedactOps};

const SECRET_FLAG_NAMES: &[&str] = &[
    "token",
    "password",
    "passwd",
    "api-key",
    "apikey",
    "secret",
    "access-key",
    "auth-token",
];

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn is_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'~' | b'+' | b'/' | b'=' | b'-')
}

fn match_bearer_or_basic(text: &str, bytes: &[u8], i: usize) -> Option<(usize, usize)> {
    let tail = text.get(i..)?;
    if tail
        .get(..6)
        .map(|s| s.eq_ignore_ascii_case("bearer"))
        .unwrap_or(false)
        && bytes
            .get(i + 6)
            .map(|b| b.is_ascii_whitespace())
            .unwrap_or(false)
    {
        let tok_start = RedactOps::scan_while(bytes, i + 6, |b| b.is_ascii_whitespace());
        let tok_end = RedactOps::scan_while(bytes, tok_start, is_token_byte);
        if tok_end - tok_start >= 12 {
            return Some((tok_start, tok_end));
        }
    }
    if tail.len() >= 19
        && tail
            .get(..13)
            .map(|s| s.eq_ignore_ascii_case("authorization"))
            .unwrap_or(false)
    {
        let mut p = i + 13;
        if matches!(bytes.get(p), Some(b'"' | b'\'')) {
            p += 1;
        }
        p = RedactOps::scan_while(bytes, p, |b| b == b' ');
        if matches!(bytes.get(p), Some(b':' | b'=')) {
            p += 1;
            p = RedactOps::scan_while(bytes, p, |b| b == b' ');
            if matches!(bytes.get(p), Some(b'"' | b'\'')) {
                p += 1;
            }
            if text
                .get(p..p + 5)
                .map(|s| s.eq_ignore_ascii_case("basic"))
                .unwrap_or(false)
                && bytes.get(p + 5) == Some(&b' ')
            {
                let b64_start = RedactOps::scan_while(bytes, p + 5, |b| b == b' ');
                let b64_end = RedactOps::scan_while(bytes, b64_start, |b| {
                    b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'='
                });
                if b64_end - b64_start >= 4
                    && !bytes.get(b64_end).map(|b| is_token_byte(*b)).unwrap_or(false)
                {
                    return Some((b64_start, b64_end));
                }
            }
        }
    }
    None
}

impl PatternScanners for RedactOps {
    fn left_word_boundary(bytes: &[u8], idx: usize) -> bool {
        idx == 0 || bytes.get(idx - 1).map(|b| !is_word_byte(*b)).unwrap_or(true)
    }

    fn right_word_boundary(bytes: &[u8], idx: usize) -> bool {
        bytes.get(idx).map(|b| !is_word_byte(*b)).unwrap_or(true)
    }

    fn scan_while(bytes: &[u8], mut i: usize, pred: fn(u8) -> bool) -> usize {
        while let Some(&b) = bytes.get(i) {
            if !pred(b) {
                break;
            }
            i += 1;
        }
        i
    }

    fn find_pem_span(text: &str, from: usize) -> Option<(usize, usize, usize, usize)> {
        let rel = text.get(from..)?.find("-----BEGIN ")?;
        let open_start = from + rel;
        let after_begin = open_start + "-----BEGIN ".len();
        let end_marker = text.get(after_begin..)?.find("PRIVATE KEY-----")?;
        let label = text.get(after_begin..after_begin + end_marker)?;
        if !label.bytes().all(|b| b.is_ascii_uppercase() || b == b' ') {
            return None;
        }
        let body_start = after_begin + end_marker + "PRIVATE KEY-----".len();
        let close_tag = format!("-----END {label}PRIVATE KEY-----");
        let close_rel = text.get(body_start..)?.find(&close_tag)?;
        let body_end = body_start + close_rel;
        let close_end = body_end + close_tag.len();
        Some((open_start, body_start, body_end, close_end))
    }

    fn redact_pem_blocks(text: &str) -> String {
        let mut out = String::new();
        let mut pos = 0;
        while let Some((_, body_s, body_e, close_e)) = Self::find_pem_span(text, pos) {
            out.push_str(text.get(pos..body_s).unwrap_or_default());
            out.push('\n');
            out.push_str(&mask_secret(text.get(body_s..body_e).unwrap_or_default().trim()));
            out.push('\n');
            out.push_str(text.get(body_e..close_e).unwrap_or_default());
            pos = close_e;
        }
        if pos == 0 {
            return text.to_string();
        }
        out.push_str(text.get(pos..).unwrap_or_default());
        out
    }

    fn redact_auth_headers(text: &str) -> String {
        let bytes = text.as_bytes();
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        let mut i = 0;
        while i < bytes.len() {
            if text.is_char_boundary(i) && Self::left_word_boundary(bytes, i) {
                if let Some((tok_start, tok_end)) = match_bearer_or_basic(text, bytes, i) {
                    out.push_str(text.get(last..tok_start).unwrap_or_default());
                    out.push_str(&mask_secret(text.get(tok_start..tok_end).unwrap_or_default()));
                    last = tok_end;
                    i = tok_end;
                    continue;
                }
            }
            i += 1;
        }
        if last == 0 {
            return text.to_string();
        }
        out.push_str(text.get(last..).unwrap_or_default());
        out
    }

    fn redact_key_assignments(text: &str) -> String {
        let bytes = text.as_bytes();
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        let mut i = 0;
        while i < bytes.len() {
            if text.is_char_boundary(i)
                && Self::left_word_boundary(bytes, i)
                && bytes
                    .get(i)
                    .map(|b| b.is_ascii_alphabetic() || *b == b'_')
                    .unwrap_or(false)
            {
                let key_end = Self::scan_while(bytes, i, |b| {
                    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
                });
                let key = text.get(i..key_end).unwrap_or_default();
                if is_secret_key_name(key) {
                    let mut p = key_end;
                    if matches!(bytes.get(p), Some(b'"' | b'\'')) {
                        p += 1;
                    }
                    p = Self::scan_while(bytes, p, |b| b == b' ' || b == b'\t');
                    if let Some(&sep_char) = bytes.get(p).filter(|b| matches!(b, b'=' | b':')) {
                        p = Self::scan_while(bytes, p + 1, |b| b == b' ' || b == b'\t');
                        if matches!(bytes.get(p), Some(b'"' | b'\'')) {
                            p += 1;
                        }
                        let val_start = p;
                        let val_end = Self::scan_while(bytes, val_start, is_token_byte);
                        let suffix_key_eq = sep_char == b'='
                            && (key.to_ascii_lowercase().ends_with("_key")
                                || key.to_ascii_lowercase().ends_with("-key"));
                        let min_len = if suffix_key_eq { 1 } else { 8 };
                        if val_end - val_start >= min_len {
                            out.push_str(text.get(last..val_start).unwrap_or_default());
                            out.push_str(&mask_secret(
                                text.get(val_start..val_end).unwrap_or_default(),
                            ));
                            last = val_end;
                            i = val_end;
                            continue;
                        }
                    }
                }
            }
            i += 1;
        }
        if last == 0 {
            return text.to_string();
        }
        out.push_str(text.get(last..).unwrap_or_default());
        out
    }

    fn redact_secret_flags(text: &str) -> String {
        let bytes = text.as_bytes();
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        let mut i = 0;
        while i < bytes.len() {
            if bytes.get(i..i + 2) == Some(b"--") {
                let name_start = i + 2;
                let name_end =
                    Self::scan_while(bytes, name_start, |b| b.is_ascii_alphanumeric() || b == b'-');
                let flag_name = text
                    .get(name_start..name_end)
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if SECRET_FLAG_NAMES.contains(&flag_name.as_str()) {
                    let mut p = name_end;
                    let has_sep = if bytes.get(p) == Some(&b'=') {
                        p += 1;
                        true
                    } else if bytes.get(p).map(|b| b.is_ascii_whitespace()).unwrap_or(false) {
                        p = Self::scan_while(bytes, p, |b| b.is_ascii_whitespace());
                        true
                    } else {
                        false
                    };
                    if has_sep {
                        if matches!(bytes.get(p), Some(b'"' | b'\'')) {
                            p += 1;
                        }
                        if bytes.get(p) != Some(&b'-') {
                            let val_end = Self::scan_while(bytes, p, is_token_byte);
                            if val_end > p {
                                out.push_str(text.get(last..p).unwrap_or_default());
                                out.push_str(&mask_secret(text.get(p..val_end).unwrap_or_default()));
                                last = val_end;
                                i = val_end;
                                continue;
                            }
                        }
                    }
                }
            }
            i += 1;
        }
        if last == 0 {
            return text.to_string();
        }
        out.push_str(text.get(last..).unwrap_or_default());
        out
    }

    fn redact_url_userinfo(text: &str) -> String {
        let bytes = text.as_bytes();
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        let mut i = 0;
        while let Some(rel) = text.get(i..).and_then(|s| s.find("://")) {
            let sep = i + rel;
            if sep > 0
                && bytes
                    .get(sep - 1)
                    .map(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'.' | b'-'))
                    .unwrap_or(false)
            {
                let after_scheme = sep + 3;
                let authority_end = Self::scan_while(bytes, after_scheme, |b| {
                    !b.is_ascii_whitespace() && !matches!(b, b'/' | b'\'' | b'"')
                });
                if let Some(auth) = text.get(after_scheme..authority_end) {
                    if let Some(at_rel) = auth.find('@') {
                        if let Some(colon_rel) = auth.get(..at_rel).unwrap_or_default().find(':') {
                            let sec_start = after_scheme + colon_rel + 1;
                            let sec_end = after_scheme + at_rel;
                            if let Some(secret) = text.get(sec_start..sec_end) {
                                if !secret.is_empty() && !secret.contains('«') && !secret.contains('»')
                                {
                                    out.push_str(text.get(last..sec_start).unwrap_or_default());
                                    out.push_str(&mask_secret(secret));
                                    last = sec_end;
                                    i = sec_end;
                                    continue;
                                }
                            }
                        }
                    }
                }
            }
            i = sep + 3;
        }
        if last == 0 {
            return text.to_string();
        }
        out.push_str(text.get(last..).unwrap_or_default());
        out
    }
}

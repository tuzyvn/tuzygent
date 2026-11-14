use super::{mask_secret, PatternScanners, PrefixScanners, RedactOps};

impl PrefixScanners for RedactOps {
    fn match_known_credential_prefix(text: &str, idx: usize) -> Option<usize> {
        let tail = text.get(idx..)?;
        let bytes = text.as_bytes();
        let token_end = |start: usize| {
            Self::scan_while(bytes, start, |b| {
                b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
            })
        };
        let alnum_end = |start: usize| Self::scan_while(bytes, start, |b| b.is_ascii_alphanumeric());

        for prefix in ["sk-ant-", "sk-proj-", "sk-live-", "sk-test-", "sk-"] {
            if tail.starts_with(prefix) {
                let start = idx + prefix.len();
                let end = token_end(start);
                if end - start >= 16 {
                    return Some(end);
                }
            }
        }
        for prefix in ["sk_live_", "sk_test_", "rk_live_", "rk_test_"] {
            if tail.starts_with(prefix) {
                let start = idx + prefix.len();
                let end = alnum_end(start);
                if end - start >= 16 {
                    return Some(end);
                }
            }
        }
        for (prefix, min_len, alnum_only) in [
            ("xai-", 20usize, false),
            ("gsk_", 40, true),
            ("csk-", 32, true),
            ("hf_", 30, true),
            ("pypi-AgE", 82, false),
            ("ghp_", 20, true),
            ("gho_", 20, true),
            ("ghu_", 20, true),
            ("ghs_", 20, true),
            ("ghr_", 20, true),
            ("github_pat_", 20, false),
            ("AIza", 30, false),
            ("npm_", 20, true),
            ("whsec_", 20, false),
            ("bak_", 20, false),
        ] {
            if tail.starts_with(prefix) {
                let start = idx + prefix.len();
                let end = if alnum_only {
                    alnum_end(start)
                } else {
                    token_end(start)
                };
                if end - start >= min_len {
                    return Some(end);
                }
            }
        }
        for (prefix, exact_len) in [("ntn_", 46usize), ("pplx-", 48usize)] {
            if tail.starts_with(prefix) {
                let start = idx + prefix.len();
                let end = alnum_end(start);
                if end - start == exact_len
                    && Self::right_word_boundary(bytes, end)
                    && bytes.get(end) != Some(&b'-')
                {
                    return Some(end);
                }
            }
        }
        if tail.starts_with("glpat-")
            || tail.starts_with("gldt-")
            || tail.starts_with("glrt-")
            || tail.starts_with("glrtr-")
        {
            if let Some(dash) = tail.find('-') {
                let start = idx + dash + 1;
                let end = Self::scan_while(bytes, start, |b| {
                    b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.'
                });
                if end - start >= 20 {
                    return Some(end);
                }
            }
        }
        if tail.starts_with("LLM|") {
            let num_start = idx + 4;
            let num_end = Self::scan_while(bytes, num_start, |b| b.is_ascii_digit());
            if num_end - num_start >= 6 && bytes.get(num_end) == Some(&b'|') {
                let tok_start = num_end + 1;
                let tok_end = token_end(tok_start);
                if tok_end - tok_start >= 16 {
                    return Some(tok_end);
                }
            }
        }
        if (tail.starts_with("AKIA") || tail.starts_with("ASIA")) && tail.len() >= 20 {
            let seg_end = idx + 20;
            if bytes
                .get(idx + 4..seg_end)
                .map(|s| s.iter().all(|b| b.is_ascii_digit() || b.is_ascii_uppercase()))
                .unwrap_or(false)
                && Self::right_word_boundary(bytes, seg_end)
            {
                return Some(seg_end);
            }
        }
        if tail.starts_with("xox")
            && bytes
                .get(idx + 3)
                .map(|b| matches!(b, b'a' | b'b' | b'p' | b'o' | b's' | b'r'))
                .unwrap_or(false)
            && bytes.get(idx + 4) == Some(&b'-')
        {
            let end = Self::scan_while(bytes, idx + 5, |b| b.is_ascii_alphanumeric() || b == b'-');
            if end - (idx + 5) >= 20 {
                return Some(end);
            }
        }
        if tail.starts_with("xapp-")
            && bytes.get(idx + 5).map(|b| b.is_ascii_digit()).unwrap_or(false)
            && bytes.get(idx + 6) == Some(&b'-')
        {
            let end = Self::scan_while(bytes, idx + 7, |b| b.is_ascii_alphanumeric() || b == b'-');
            if end - (idx + 7) >= 20 {
                return Some(end);
            }
        }
        if tail.starts_with("eyJ") {
            let p1 = token_end(idx);
            if bytes.get(p1) == Some(&b'.') {
                let p2 = token_end(p1 + 1);
                if p1 - idx >= 11 && p2 - (p1 + 1) >= 8 && bytes.get(p2) == Some(&b'.') {
                    let p3 = token_end(p2 + 1);
                    if p3 - (p2 + 1) >= 8 && Self::right_word_boundary(bytes, p3) {
                        return Some(p3);
                    }
                }
            }
        }
        if tail.starts_with("SG.") {
            let p1 = token_end(idx + 3);
            if p1 - (idx + 3) == 22 && bytes.get(p1) == Some(&b'.') {
                let p2 = token_end(p1 + 1);
                if p2 - (p1 + 1) == 43 && Self::right_word_boundary(bytes, p2) {
                    return Some(p2);
                }
            }
        }
        None
    }

    fn redact_known_prefixes(text: &str) -> String {
        let bytes = text.as_bytes();
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        let mut i = 0;
        while i < bytes.len() {
            if text.is_char_boundary(i) && Self::left_word_boundary(bytes, i) {
                if let Some(end) = Self::match_known_credential_prefix(text, i) {
                    out.push_str(text.get(last..i).unwrap_or_default());
                    out.push_str(&mask_secret(text.get(i..end).unwrap_or_default()));
                    last = end;
                    i = end;
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
}

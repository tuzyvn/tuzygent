use agent_domain::VerificationSpec;

pub(super) fn spec_description(spec: &VerificationSpec) -> String {
    match spec {
        VerificationSpec::Command { command, .. } => format!("Command '{}'", command),
        VerificationSpec::FileExists { path, .. } => format!("FileExists '{}'", path),
        VerificationSpec::JsonValid { path } => format!("JsonValid '{}'", path),
        VerificationSpec::ContentContains { path, pattern } => {
            format!("ContentContains in '{}' pattern '{}'", path, pattern)
        }
        VerificationSpec::Custom { description } => format!("Custom '{}'", description),
    }
}

pub(super) fn take_tail_chars(s: &str, n: usize) -> String {
    let count = s.chars().count();
    if count > n {
        s.chars().skip(count - n).collect()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{spec_description, take_tail_chars};
    use agent_domain::VerificationSpec;

    #[test]
    fn spec_description_formats_each_verification_spec_variant() {
        let cases = [
            (
                VerificationSpec::Command {
                    command: "cargo test".into(),
                    expected_exit_code: 0,
                },
                "Command 'cargo test'",
            ),
            (
                VerificationSpec::FileExists {
                    path: "output.txt".into(),
                    min_bytes: Some(1),
                },
                "FileExists 'output.txt'",
            ),
            (
                VerificationSpec::JsonValid {
                    path: "result.json".into(),
                },
                "JsonValid 'result.json'",
            ),
            (
                VerificationSpec::ContentContains {
                    path: "report.txt".into(),
                    pattern: "complete".into(),
                },
                "ContentContains in 'report.txt' pattern 'complete'",
            ),
            (
                VerificationSpec::Custom {
                    description: "manual review".into(),
                },
                "Custom 'manual review'",
            ),
        ];

        for (spec, expected) in cases {
            assert_eq!(spec_description(&spec), expected);
        }
    }

    #[test]
    fn take_tail_chars_handles_empty_input() {
        assert_eq!(take_tail_chars("", 3), "");
    }

    #[test]
    fn take_tail_chars_returns_input_shorter_than_limit() {
        assert_eq!(take_tail_chars("abc", 4), "abc");
    }

    #[test]
    fn take_tail_chars_returns_input_at_exact_limit() {
        assert_eq!(take_tail_chars("abc", 3), "abc");
    }

    #[test]
    fn take_tail_chars_truncates_to_the_last_n_characters() {
        assert_eq!(take_tail_chars("abcdef", 3), "def");
    }

    #[test]
    fn take_tail_chars_counts_unicode_characters() {
        assert_eq!(take_tail_chars("aé🙂界", 2), "🙂界");
    }
}

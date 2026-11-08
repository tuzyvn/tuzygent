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

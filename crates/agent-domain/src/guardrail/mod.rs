mod benchmark;
mod fault;
mod policy;
mod resilience;

pub use benchmark::*;
pub use fault::*;
pub use policy::*;
pub use resilience::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_each_alternative_and_normalizes_whitespace_and_case() {
        let policy = GuardrailPolicy::default_enterprise();
        for command in [
            "DROP TABLE users",
            "truncate users",
            "chmod   777 file",
            "chown root file",
            "rm -rf data",
        ] {
            assert!(
                policy.check_command_safety(command).is_err(),
                "allowed {command}"
            );
        }
        assert!(policy.check_command_safety("cargo test").is_ok());
    }
}

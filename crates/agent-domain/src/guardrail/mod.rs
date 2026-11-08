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
    fn default_policy_has_expected_limits_and_no_red_lines() {
        let policy = GuardrailPolicy::default();

        assert!(policy.red_lines.is_empty());
        assert_eq!(policy.max_subtasks_per_goal, 30);
        assert_eq!(policy.max_retries_on_error, 3);
        assert!(policy.allow_shell_execution);
        assert_eq!(policy.max_output_bytes, 10 * 1024 * 1024);
    }

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

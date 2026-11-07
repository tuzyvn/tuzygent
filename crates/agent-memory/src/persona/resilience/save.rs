use super::*;
use crate::persona::fault::{PersonaError, PersonaFault};

impl ResilientPersonaLoader {
    pub fn save_profile_cached(
        &mut self,
        profile: &AgentPersonaProfile,
    ) -> Result<(), PersonaError> {
        if profile.identity.name.trim().is_empty() {
            self.rejected_count += 1;
            return Err(PersonaError::InvalidIdentity(
                "Persona name cannot be empty".to_string(),
            ));
        }

        let mut attempt = 0;
        loop {
            attempt += 1;
            if self.check_fault_injection(attempt)?.is_some() {
                continue;
            }
            self.saved_count += 1;
            self.record_profile_cached(profile);
            return Ok(());
        }
    }

    pub fn save_batch(
        &mut self,
        profiles: &[AgentPersonaProfile],
    ) -> Vec<Result<(), PersonaError>> {
        let mut results = Vec::with_capacity(profiles.len());
        for profile in profiles {
            results.push(self.save_profile_cached(profile));
        }
        results
    }

    pub(crate) fn check_fault_injection(
        &mut self,
        attempt: usize,
    ) -> Result<Option<()>, PersonaError> {
        if let Some(fault) = self.policy.should_inject() {
            match fault {
                PersonaFault::SocketTimeout
                | PersonaFault::ConnectionReset
                | PersonaFault::TransientIo
                | PersonaFault::LockFailure => {
                    if attempt <= self.max_retries {
                        self.recovered_count += 1;
                        Ok(Some(()))
                    } else {
                        Err(fault.to_error())
                    }
                }
                PersonaFault::CorruptedProfile | PersonaFault::InvalidIdentity => {
                    self.rejected_count += 1;
                    Err(fault.to_error())
                }
            }
        } else {
            Ok(None)
        }
    }
}

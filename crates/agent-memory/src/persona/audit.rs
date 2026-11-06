use crate::persona::fault::PersonaError;
use crate::persona::resilience::ResilientPersonaLoader;
use agent_domain::{ActionSeverity, AuditEventType, AuditLedger, AgentPersonaProfile};
use std::path::Path;

impl ResilientPersonaLoader {
    pub async fn load_from_dir_and_record<P: AsRef<Path>>(
        &mut self,
        ledger: &mut AuditLedger,
        dir_path: P,
    ) -> Result<AgentPersonaProfile, PersonaError> {
        let p = dir_path.as_ref();
        match self.load_from_dir(p).await {
            Ok(profile) => {
                ledger.record_event(
                    AuditEventType::CommandAllowed,
                    ActionSeverity::Routine,
                    format!("Persona profile successfully loaded: '{}'", profile.identity.name),
                );
                Ok(profile)
            }
            Err(err) => {
                let severity = match &err {
                    PersonaError::CorruptedProfile(_) | PersonaError::InvalidIdentity(_) => {
                        ActionSeverity::Destructive
                    }
                    _ => ActionSeverity::HighImpact,
                };
                let event_type = match &err {
                    PersonaError::InvalidIdentity(_) => AuditEventType::CommandBlocked,
                    _ => AuditEventType::CommandException,
                };
                ledger.record_event(
                    event_type,
                    severity,
                    format!("Persona load failure for '{}': {}", p.display(), err),
                );
                Err(err)
            }
        }
    }

    pub async fn export_to_dir_and_record<P: AsRef<Path>>(
        &mut self,
        ledger: &mut AuditLedger,
        profile: &AgentPersonaProfile,
        dir_path: P,
    ) -> Result<(), PersonaError> {
        let p = dir_path.as_ref();
        match self.export_to_dir(profile, p).await {
            Ok(()) => {
                ledger.record_event(
                    AuditEventType::CommandAllowed,
                    ActionSeverity::Routine,
                    format!("Persona profile successfully exported: '{}'", profile.identity.name),
                );
                Ok(())
            }
            Err(err) => {
                let severity = match &err {
                    PersonaError::CorruptedProfile(_) | PersonaError::InvalidIdentity(_) => {
                        ActionSeverity::Destructive
                    }
                    _ => ActionSeverity::HighImpact,
                };
                let event_type = match &err {
                    PersonaError::InvalidIdentity(_) => AuditEventType::CommandBlocked,
                    _ => AuditEventType::CommandException,
                };
                ledger.record_event(
                    event_type,
                    severity,
                    format!("Persona export failure for '{}': {}", profile.identity.name, err),
                );
                Err(err)
            }
        }
    }
}

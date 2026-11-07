use super::*;
use crate::persona::fault::PersonaError;
use crate::persona::loader::PersonaLoader;
use std::path::Path;

impl ResilientPersonaLoader {
    pub async fn load_from_dir<P: AsRef<Path>>(
        &mut self,
        dir_path: P,
    ) -> Result<AgentPersonaProfile, PersonaError> {
        let dir = dir_path.as_ref();
        if !dir.exists() {
            self.rejected_count += 1;
            return Err(PersonaError::NotFound(dir.display().to_string()));
        }

        let mut attempt = 0;
        loop {
            attempt += 1;
            if self.check_fault_injection(attempt)?.is_some() {
                continue;
            }

            match PersonaLoader::load_from_dir(dir).await {
                Ok(profile) => {
                    if profile.identity.name.trim().is_empty() {
                        self.rejected_count += 1;
                        return Err(PersonaError::InvalidIdentity(
                            "Persona name is empty".to_string(),
                        ));
                    }
                    self.loaded_count += 1;
                    self.record_profile_cached(&profile);
                    return Ok(profile);
                }
                Err(err) => {
                    let error_text = err.to_string();
                    if error_text.contains("corrupt") || error_text.contains("Failed to parse") {
                        self.rejected_count += 1;
                        return Err(PersonaError::CorruptedProfile(error_text));
                    }
                    if attempt <= self.max_retries {
                        self.recovered_count += 1;
                        continue;
                    }
                    return Err(PersonaError::Other(error_text));
                }
            }
        }
    }

    pub async fn load_from_markdown_files<P: AsRef<Path>>(
        &mut self,
        dir_path: P,
    ) -> Result<AgentPersonaProfile, PersonaError> {
        let dir = dir_path.as_ref();
        if !dir.exists() {
            self.rejected_count += 1;
            return Err(PersonaError::NotFound(dir.display().to_string()));
        }

        let mut attempt = 0;
        loop {
            attempt += 1;
            if self.check_fault_injection(attempt)?.is_some() {
                continue;
            }

            match PersonaLoader::load_from_markdown_files(dir).await {
                Ok(profile) => {
                    if profile.identity.name.trim().is_empty() {
                        self.rejected_count += 1;
                        return Err(PersonaError::InvalidIdentity(
                            "Persona name is empty".to_string(),
                        ));
                    }
                    self.loaded_count += 1;
                    self.record_profile_cached(&profile);
                    return Ok(profile);
                }
                Err(err) => {
                    let error_text = err.to_string();
                    if error_text.contains("corrupt") || error_text.contains("Failed to parse") {
                        self.rejected_count += 1;
                        return Err(PersonaError::CorruptedProfile(error_text));
                    }
                    if attempt <= self.max_retries {
                        self.recovered_count += 1;
                        continue;
                    }
                    return Err(PersonaError::Other(error_text));
                }
            }
        }
    }

    pub async fn export_to_dir<P: AsRef<Path>>(
        &mut self,
        profile: &AgentPersonaProfile,
        dir_path: P,
    ) -> Result<(), PersonaError> {
        if profile.identity.name.trim().is_empty() {
            self.rejected_count += 1;
            return Err(PersonaError::InvalidIdentity(
                "Persona name cannot be empty".to_string(),
            ));
        }

        let dir = dir_path.as_ref();
        let mut attempt = 0;
        loop {
            attempt += 1;
            if self.check_fault_injection(attempt)?.is_some() {
                continue;
            }

            match PersonaLoader::export_to_dir(profile, dir).await {
                Ok(()) => {
                    self.saved_count += 1;
                    self.record_profile_cached(profile);
                    return Ok(());
                }
                Err(err) => {
                    if attempt <= self.max_retries {
                        self.recovered_count += 1;
                        continue;
                    }
                    return Err(PersonaError::Other(err.to_string()));
                }
            }
        }
    }
}

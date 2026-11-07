use crate::persona::fault::{PersonaError, PersonaFault};
use crate::persona::loader::PersonaLoader;
use crate::persona::policy::PersonaFaultPolicy;
use agent_domain::AgentPersonaProfile;
use std::collections::{HashSet, VecDeque};
use std::path::Path;

pub type ResilientPersona = ResilientPersonaLoader;

#[derive(Debug, Clone)]
pub struct ResilientPersonaLoader {
    policy: PersonaFaultPolicy,
    max_retries: usize,
    loaded_count: usize,
    saved_count: usize,
    recovered_count: usize,
    rejected_count: usize,
    seen_personas: HashSet<String>,
    persona_queue: VecDeque<String>,
    cached_profiles: VecDeque<AgentPersonaProfile>,
    max_capacity: Option<usize>,
}

impl Default for ResilientPersonaLoader {
    fn default() -> Self {
        Self {
            policy: PersonaFaultPolicy::none(),
            max_retries: 3,
            loaded_count: 0,
            saved_count: 0,
            recovered_count: 0,
            rejected_count: 0,
            seen_personas: HashSet::new(),
            persona_queue: VecDeque::new(),
            cached_profiles: VecDeque::new(),
            max_capacity: None,
        }
    }
}

impl ResilientPersonaLoader {
    pub fn new(policy: PersonaFaultPolicy, max_retries: usize) -> Self {
        Self {
            policy,
            max_retries,
            loaded_count: 0,
            saved_count: 0,
            recovered_count: 0,
            rejected_count: 0,
            seen_personas: HashSet::new(),
            persona_queue: VecDeque::new(),
            cached_profiles: VecDeque::new(),
            max_capacity: None,
        }
    }

    pub fn with_policy(mut self, policy: PersonaFaultPolicy) -> Self {
        self.policy = policy;
        self
    }

    pub fn with_max_retries(mut self, max_retries: usize) -> Self {
        self.max_retries = max_retries;
        self
    }

    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.seen_personas.reserve(capacity);
        self.persona_queue.reserve(capacity);
        self.cached_profiles.reserve(capacity);
        self
    }

    pub fn with_max_capacity(mut self, max_capacity: usize) -> Self {
        self.max_capacity = Some(max_capacity);
        self.seen_personas.reserve(max_capacity);
        self.persona_queue.reserve(max_capacity);
        self.cached_profiles.reserve(max_capacity);
        self
    }

    pub fn policy(&self) -> &PersonaFaultPolicy {
        &self.policy
    }

    pub fn policy_mut(&mut self) -> &mut PersonaFaultPolicy {
        &mut self.policy
    }

    pub fn set_policy(&mut self, policy: PersonaFaultPolicy) {
        self.policy = policy;
    }

    pub fn max_retries(&self) -> usize {
        self.max_retries
    }

    pub fn set_max_retries(&mut self, max_retries: usize) {
        self.max_retries = max_retries;
    }

    pub fn loaded_count(&self) -> usize {
        self.loaded_count
    }

    pub fn saved_count(&self) -> usize {
        self.saved_count
    }

    pub fn recovered_count(&self) -> usize {
        self.recovered_count
    }

    pub fn rejected_count(&self) -> usize {
        self.rejected_count
    }

    pub fn reset_counts(&mut self) {
        self.loaded_count = 0;
        self.saved_count = 0;
        self.recovered_count = 0;
        self.rejected_count = 0;
        self.seen_personas.clear();
        self.persona_queue.clear();
        self.cached_profiles.clear();
    }

    pub fn memory_usage_bytes(&self) -> usize {
        let set_mem = self.seen_personas.capacity() * (std::mem::size_of::<String>() + 40);
        let queue_mem = self.persona_queue.capacity() * (std::mem::size_of::<String>() + 40);
        let profile_mem =
            self.cached_profiles.capacity() * std::mem::size_of::<AgentPersonaProfile>();
        std::mem::size_of::<Self>() + set_mem + queue_mem + profile_mem
    }

    pub fn cached_profiles(&self) -> &VecDeque<AgentPersonaProfile> {
        &self.cached_profiles
    }

    pub fn has_persona(&self, name: &str) -> bool {
        self.seen_personas.contains(name)
    }

    pub fn get_cached_profile(&self, name: &str) -> Option<&AgentPersonaProfile> {
        if !self.has_persona(name) {
            return None;
        }
        self.cached_profiles
            .iter()
            .find(|p| p.identity.name == name)
    }

    pub fn record_profile_cached(&mut self, profile: &AgentPersonaProfile) {
        if self.seen_personas.contains(&profile.identity.name) {
            return;
        }
        if let Some(max_cap) = self.max_capacity {
            while self.persona_queue.len() >= max_cap {
                if let Some(oldest_name) = self.persona_queue.pop_front() {
                    self.seen_personas.remove(&oldest_name);
                }
                self.cached_profiles.pop_front();
            }
            self.persona_queue.push_back(profile.identity.name.clone());
            self.cached_profiles.push_back(profile.clone());
        }
        self.seen_personas.insert(profile.identity.name.clone());
    }

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
            if let Some(()) = self.check_fault_injection(attempt)? {
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
        for p in profiles {
            results.push(self.save_profile_cached(p));
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
            if let Some(()) = self.check_fault_injection(attempt)? {
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
                    let err_str = err.to_string();
                    if err_str.contains("corrupt") || err_str.contains("Failed to parse") {
                        self.rejected_count += 1;
                        return Err(PersonaError::CorruptedProfile(err_str));
                    }
                    if attempt <= self.max_retries {
                        self.recovered_count += 1;
                        continue;
                    }
                    return Err(PersonaError::Other(err_str));
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
            if let Some(()) = self.check_fault_injection(attempt)? {
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
                    let err_str = err.to_string();
                    if err_str.contains("corrupt") || err_str.contains("Failed to parse") {
                        self.rejected_count += 1;
                        return Err(PersonaError::CorruptedProfile(err_str));
                    }
                    if attempt <= self.max_retries {
                        self.recovered_count += 1;
                        continue;
                    }
                    return Err(PersonaError::Other(err_str));
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
            if let Some(()) = self.check_fault_injection(attempt)? {
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

use crate::persona::policy::PersonaFaultPolicy;
use agent_domain::AgentPersonaProfile;
use std::collections::{HashSet, VecDeque};

mod cache;
mod io;
mod save;

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
}

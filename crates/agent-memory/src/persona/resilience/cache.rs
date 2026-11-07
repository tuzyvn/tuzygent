use super::*;

impl ResilientPersonaLoader {
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
            .find(|profile| profile.identity.name == name)
    }

    pub fn record_profile_cached(&mut self, profile: &AgentPersonaProfile) {
        if self.seen_personas.contains(&profile.identity.name) {
            return;
        }
        if let Some(max_capacity) = self.max_capacity {
            while self.persona_queue.len() >= max_capacity {
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
}

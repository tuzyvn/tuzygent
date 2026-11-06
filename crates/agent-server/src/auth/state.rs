use axum::http::HeaderValue;
use std::sync::{Arc, Mutex};

use super::cache::ShardedAuthCache;
use super::fault::AuthFaultPolicy;

/// Shared application authentication state passed to Axum handlers and middleware.
#[derive(Clone)]
pub struct AuthState {
    pub token: Arc<String>,
    pub fallback_token: Option<Arc<String>>,
    pub allowed_origins: Vec<HeaderValue>,
    pub fault_policy: Arc<Mutex<AuthFaultPolicy>>,
    pub cache: Option<Arc<ShardedAuthCache>>,
}

impl AuthState {
    pub fn new(token: Arc<String>, allowed_origins: Vec<HeaderValue>) -> Self {
        Self {
            token,
            fallback_token: None,
            allowed_origins,
            fault_policy: Arc::new(Mutex::new(AuthFaultPolicy::none())),
            cache: None,
        }
    }

    pub fn with_fault_policy(mut self, policy: AuthFaultPolicy) -> Self {
        self.fault_policy = Arc::new(Mutex::new(policy));
        self
    }

    pub fn with_fallback_token(mut self, fallback: Arc<String>) -> Self {
        self.fallback_token = Some(fallback);
        self
    }

    pub fn with_cache(mut self, capacity: usize) -> Self {
        self.cache = Some(Arc::new(ShardedAuthCache::with_default_shards(capacity)));
        self
    }

    pub fn with_shared_cache(mut self, cache: Arc<ShardedAuthCache>) -> Self {
        self.cache = Some(cache);
        self
    }

    pub fn cache(&self) -> Option<&Arc<ShardedAuthCache>> {
        self.cache.as_ref()
    }
}

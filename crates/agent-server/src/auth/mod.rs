pub mod benchmark;
pub mod cache;
pub mod extractors;
pub mod fault;
pub mod middleware;
pub mod sharded;
pub mod state;
pub mod types;
pub mod validator;

pub use benchmark::{AuthBenchmarkHarness, AuthBenchmarkStats};
pub use cache::{AuthCacheEntry, AuthCacheStats, ConcurrentAuthCache, ShardedAuthCache};
pub use extractors::{extract_bearer_token, extract_ws_token, token_matches};
pub use fault::{simulate_auth_fault, AuthFault, AuthFaultPolicy};
pub use middleware::require_auth;
pub use sharded::ShardedResilientAuthValidator;
pub use state::AuthState;
pub use types::{AuthError, AuthErrorTier};
pub use validator::{AuthFallbackStrategy, ResilientAuthValidator};

use crate::persona::fault::PersonaError;
use crate::persona::resilience::ResilientPersonaLoader;
use agent_domain::{
    AgentId, AgentIdentity, AgentMission, AgentPersonaProfile, AgentRole, AgentSoul, AutonomyLevel,
};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub struct PersonaBenchmarkStats {
    pub total_requests: usize,
    pub successful_operations: usize,
    pub rejected_count: usize,
    pub p50_latency_us: f64,
    pub p90_latency_us: f64,
    pub p99_latency_us: f64,
    pub max_latency_us: f64,
    pub total_duration: Duration,
    pub throughput_ops_per_sec: f64,
    pub memory_allocated_bytes: usize,
}

pub struct PersonaBenchmarkHarness;

impl PersonaBenchmarkHarness {
    fn compute_stats(
        mut latencies: Vec<f64>,
        total_requests: usize,
        successful: usize,
        rejected: usize,
        duration: Duration,
        mem: usize,
    ) -> PersonaBenchmarkStats {
        latencies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let p50 = Self::percentile(&latencies, 0.50);
        let p90 = Self::percentile(&latencies, 0.90);
        let p99 = Self::percentile(&latencies, 0.99);
        let max = latencies.last().copied().unwrap_or(0.0);
        let secs = duration.as_secs_f64();
        let throughput = if secs > 0.0 {
            total_requests as f64 / secs
        } else {
            0.0
        };
        PersonaBenchmarkStats {
            total_requests,
            successful_operations: successful,
            rejected_count: rejected,
            p50_latency_us: p50,
            p90_latency_us: p90,
            p99_latency_us: p99,
            max_latency_us: max,
            total_duration: duration,
            throughput_ops_per_sec: throughput,
            memory_allocated_bytes: mem,
        }
    }

    pub fn make_benchmark_profile(idx: usize, is_invalid: bool) -> AgentPersonaProfile {
        if is_invalid {
            AgentPersonaProfile {
                identity: AgentIdentity {
                    id: AgentId::from_str("invalid-000"),
                    name: String::new(),
                    description: String::new(),
                    autonomy_level: AutonomyLevel::Full,
                    created_at: chrono::Utc::now(),
                },
                role: AgentRole {
                    role_code: "INVALID".to_string(),
                    title: String::new(),
                    primary_domain: String::new(),
                    capabilities: Vec::new(),
                },
                soul: AgentSoul {
                    persona: String::new(),
                    tone_of_voice: String::new(),
                    core_values: Vec::new(),
                    voice_profile: None,
                },
                mission: AgentMission {
                    high_level_objective: String::new(),
                    kpis: Vec::new(),
                    constraints: Vec::new(),
                },
            }
        } else {
            let name = format!("Persona-Agent-{:06}", idx);
            AgentPersonaProfile {
                identity: AgentIdentity {
                    id: AgentId::from_str(&format!("agent-{:06}", idx)),
                    name: name.clone(),
                    description: format!("Autonomous agent for workload item {}", idx),
                    autonomy_level: AutonomyLevel::Full,
                    created_at: chrono::Utc::now(),
                },
                role: AgentRole {
                    role_code: format!("ROLE-{:04}", idx % 100),
                    title: format!("Specialist {}", idx),
                    primary_domain: "Distributed Agent Memory & Systems".to_string(),
                    capabilities: vec![
                        "memory persistence".to_string(),
                        "concurrency control".to_string(),
                    ],
                },
                soul: AgentSoul {
                    persona: "Deterministic, ultra-fast and concurrent-safe.".to_string(),
                    tone_of_voice: "Precise and resilient".to_string(),
                    core_values: vec![
                        "Lock-free Concurrency".to_string(),
                        "Zero-copy Caching".to_string(),
                    ],
                    voice_profile: None,
                },
                mission: AgentMission {
                    high_level_objective: format!(
                        "Handle 10,000 concurrent requests for persona {}",
                        idx
                    ),
                    kpis: vec![
                        "P99 latency < 20ms".to_string(),
                        "Zero lock starvation".to_string(),
                    ],
                    constraints: vec!["Strict memory bounding".to_string()],
                },
            }
        }
    }

    pub fn run_benchmark(
        loader: &mut ResilientPersonaLoader,
        num_requests: usize,
    ) -> PersonaBenchmarkStats {
        let mut latencies_us = Vec::with_capacity(num_requests);
        let start = Instant::now();
        let mut successful = 0;
        let mut rejected = 0;

        for i in 0..num_requests {
            let is_invalid = i.is_multiple_of(10);
            let profile = Self::make_benchmark_profile(i, is_invalid);
            let t0 = Instant::now();
            match loader.save_profile_cached(&profile) {
                Ok(_) => successful += 1,
                Err(PersonaError::InvalidIdentity(_)) => rejected += 1,
                Err(_) => {}
            }
            latencies_us.push(t0.elapsed().as_secs_f64() * 1_000_000.0);
        }

        Self::compute_stats(
            latencies_us,
            num_requests,
            successful,
            rejected,
            start.elapsed(),
            loader.memory_usage_bytes(),
        )
    }

    pub fn run_single_lock_benchmark(
        num_requests: usize,
        concurrency: usize,
    ) -> PersonaBenchmarkStats {
        let shared_loader = Arc::new(Mutex::new(
            ResilientPersonaLoader::default().with_max_capacity(num_requests + 64),
        ));

        let start = Instant::now();
        let per_thread = num_requests / concurrency;
        let mut handles = Vec::with_capacity(concurrency);

        for t_idx in 0..concurrency {
            let loader_clone = Arc::clone(&shared_loader);
            handles.push(std::thread::spawn(move || {
                let mut local_latencies = Vec::with_capacity(per_thread);
                let mut local_success = 0;
                let mut local_rejected = 0;

                for i in 0..per_thread {
                    let req_idx = t_idx * per_thread + i;
                    let is_invalid = req_idx.is_multiple_of(10);
                    let profile = Self::make_benchmark_profile(req_idx, is_invalid);

                    let t0 = Instant::now();
                    let res = {
                        loader_clone
                            .lock()
                            .expect("lock acquired")
                            .save_profile_cached(&profile)
                    };
                    match res {
                        Ok(_) => local_success += 1,
                        Err(PersonaError::InvalidIdentity(_)) => local_rejected += 1,
                        Err(_) => {}
                    }
                    local_latencies.push(t0.elapsed().as_secs_f64() * 1_000_000.0);
                }
                (local_latencies, local_success, local_rejected)
            }));
        }

        let mut all_latencies = Vec::with_capacity(num_requests);
        let mut total_success = 0;
        let mut total_rejected = 0;
        for handle in handles {
            if let Ok((latencies, success, rejected)) = handle.join() {
                all_latencies.extend(latencies);
                total_success += success;
                total_rejected += rejected;
            }
        }

        let mem_usage = shared_loader
            .lock()
            .expect("lock acquired")
            .memory_usage_bytes();

        Self::compute_stats(
            all_latencies,
            num_requests,
            total_success,
            total_rejected,
            start.elapsed(),
            mem_usage,
        )
    }

    pub fn run_concurrent_benchmark(
        num_requests: usize,
        concurrency: usize,
    ) -> PersonaBenchmarkStats {
        const NUM_SHARDS: usize = 16;
        let per_shard_cap = (num_requests / NUM_SHARDS) + 64;
        let shards: Arc<Vec<RwLock<ResilientPersonaLoader>>> = Arc::new(
            (0..NUM_SHARDS)
                .map(|_| {
                    RwLock::new(ResilientPersonaLoader::default().with_max_capacity(per_shard_cap))
                })
                .collect(),
        );

        let start = Instant::now();
        let per_thread = num_requests / concurrency;
        let mut handles = Vec::with_capacity(concurrency);

        for t_idx in 0..concurrency {
            let shards_clone = Arc::clone(&shards);
            handles.push(std::thread::spawn(move || {
                let mut local_latencies = Vec::with_capacity(per_thread);
                let mut local_success = 0;
                let mut local_rejected = 0;

                for i in 0..per_thread {
                    let req_idx = t_idx * per_thread + i;
                    let is_invalid = req_idx.is_multiple_of(10);
                    let profile = Self::make_benchmark_profile(req_idx, is_invalid);

                    let mut hasher = DefaultHasher::new();
                    req_idx.hash(&mut hasher);
                    let shard_idx = (hasher.finish() as usize) % NUM_SHARDS;

                    let t0 = Instant::now();
                    let res = {
                        shards_clone[shard_idx]
                            .write()
                            .expect("write lock acquired")
                            .save_profile_cached(&profile)
                    };
                    match res {
                        Ok(_) => local_success += 1,
                        Err(PersonaError::InvalidIdentity(_)) => local_rejected += 1,
                        Err(_) => {}
                    }
                    local_latencies.push(t0.elapsed().as_secs_f64() * 1_000_000.0);
                }
                (local_latencies, local_success, local_rejected)
            }));
        }

        let mut all_latencies = Vec::with_capacity(num_requests);
        let mut total_success = 0;
        let mut total_rejected = 0;
        for handle in handles {
            if let Ok((latencies, success, rejected)) = handle.join() {
                all_latencies.extend(latencies);
                total_success += success;
                total_rejected += rejected;
            }
        }

        let mem_usage: usize = shards
            .iter()
            .map(|s| s.read().expect("read lock acquired").memory_usage_bytes())
            .sum();

        Self::compute_stats(
            all_latencies,
            num_requests,
            total_success,
            total_rejected,
            start.elapsed(),
            mem_usage,
        )
    }

    fn percentile(sorted: &[f64], pct: f64) -> f64 {
        if sorted.is_empty() {
            return 0.0;
        }
        let idx = ((sorted.len() as f64) * pct).floor() as usize;
        sorted[idx.min(sorted.len() - 1)]
    }
}

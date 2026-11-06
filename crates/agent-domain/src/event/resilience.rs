use crate::audit_ledger::{AuditEventType, AuditLedger};
use crate::event::fault::{EventError, EventFault, EventFaultPolicy};
use crate::event::InterAgentEvent;
use crate::guardrail::ActionSeverity;
use crate::identity::AgentId;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct ResilientEventProcessor {
    policy: EventFaultPolicy,
    max_retries: usize,
    seen_ids: HashSet<String>,
    id_queue: VecDeque<String>,
    max_capacity: Option<usize>,
    processed_count: usize,
    recovered_count: usize,
    dropped_count: usize,
    rejected_count: usize,
}

impl ResilientEventProcessor {
    pub fn new(policy: EventFaultPolicy, max_retries: usize) -> Self {
        Self {
            policy,
            max_retries,
            seen_ids: HashSet::new(),
            id_queue: VecDeque::new(),
            max_capacity: None,
            processed_count: 0,
            recovered_count: 0,
            dropped_count: 0,
            rejected_count: 0,
        }
    }

    pub fn with_policy(policy: EventFaultPolicy) -> Self { Self::new(policy, 3) }
    pub fn with_capacity(mut self, capacity: usize) -> Self { self.seen_ids.reserve(capacity); self }
    pub fn with_max_capacity(mut self, max_capacity: usize) -> Self {
        self.max_capacity = Some(max_capacity);
        self.seen_ids.reserve(max_capacity);
        self.id_queue.reserve(max_capacity);
        self
    }

    pub fn policy(&self) -> &EventFaultPolicy { &self.policy }
    pub fn policy_mut(&mut self) -> &mut EventFaultPolicy { &mut self.policy }
    pub fn max_retries(&self) -> usize { self.max_retries }
    pub fn set_max_retries(&mut self, max_retries: usize) { self.max_retries = max_retries; }
    pub fn seen_ids(&self) -> &HashSet<String> { &self.seen_ids }
    pub fn clear_seen_ids(&mut self) { self.seen_ids.clear(); self.id_queue.clear(); }
    pub fn reset_counts(&mut self) {
        self.processed_count = 0; self.recovered_count = 0;
        self.dropped_count = 0; self.rejected_count = 0;
    }
    pub fn processed_count(&self) -> usize { self.processed_count }
    pub fn recovered_count(&self) -> usize { self.recovered_count }
    pub fn dropped_count(&self) -> usize { self.dropped_count }
    pub fn rejected_count(&self) -> usize { self.rejected_count }

    pub fn memory_usage_bytes(&self) -> usize {
        let set_mem = self.seen_ids.capacity() * (std::mem::size_of::<String>() + 36);
        let queue_mem = self.id_queue.capacity() * (std::mem::size_of::<String>() + 36);
        std::mem::size_of::<Self>() + set_mem + queue_mem
    }

    #[inline]
    fn record_seen(&mut self, id: String) {
        if let Some(max_cap) = self.max_capacity {
            if self.seen_ids.len() >= max_cap {
                if let Some(oldest) = self.id_queue.pop_front() {
                    self.seen_ids.remove(&oldest);
                }
            }
            self.id_queue.push_back(id.clone());
        }
        self.seen_ids.insert(id);
    }

    pub fn process_event(
        &mut self,
        event: InterAgentEvent,
    ) -> Result<Option<InterAgentEvent>, EventError> {
        if let Err(err) = event.validate() {
            self.rejected_count += 1;
            return Err(err);
        }
        if self.seen_ids.contains(&event.id) {
            return Ok(None);
        }

        let mut attempt = 0;
        loop {
            attempt += 1;
            match self.policy.should_inject() {
                Some(EventFault::Dropped) => {
                    self.dropped_count += 1;
                    return Ok(None);
                }
                Some(EventFault::Duplicate) => {
                    self.record_seen(event.id.clone());
                    self.processed_count += 1;
                    return Ok(Some(event));
                }
                Some(EventFault::CorruptedPayload) => {
                    self.rejected_count += 1;
                    return Err(EventError::CorruptedPayload(
                        "Packet payload checksum failure or corrupted JSON".to_string(),
                    ));
                }
                Some(fault) => {
                    if attempt <= self.max_retries {
                        self.recovered_count += 1;
                    } else {
                        return Err(fault.to_error());
                    }
                }
                None => {
                    self.record_seen(event.id.clone());
                    self.processed_count += 1;
                    return Ok(Some(event));
                }
            }
        }
    }

    pub fn process_batch(
        &mut self,
        events: Vec<InterAgentEvent>,
    ) -> Result<Vec<InterAgentEvent>, EventError> {
        let mut processed = Vec::with_capacity(events.len());
        for event in events {
            if let Some(ev) = self.process_event(event)? {
                processed.push(ev);
            }
        }
        Ok(processed)
    }

    pub fn process_and_record(
        &mut self,
        ledger: &mut AuditLedger,
        event: InterAgentEvent,
    ) -> Result<Option<InterAgentEvent>, EventError> {
        if let Err(err) = event.validate() {
            self.rejected_count += 1;
            ledger.record_event(
                AuditEventType::CommandBlocked,
                ActionSeverity::Destructive,
                format!("Event '{}' rejected due to invalid structure: {}", event.id, err),
            );
            return Err(err);
        }
        if self.seen_ids.contains(&event.id) {
            ledger.record_event(
                AuditEventType::CommandAllowed,
                ActionSeverity::Routine,
                format!("Event '{}' deduplicated and discarded", event.id),
            );
            return Ok(None);
        }

        let mut attempt = 0;
        loop {
            attempt += 1;
            match self.policy.should_inject() {
                Some(EventFault::Dropped) => {
                    self.dropped_count += 1;
                    ledger.record_event(
                        AuditEventType::CommandBlocked,
                        ActionSeverity::Destructive,
                        format!("Event '{}' dropped in transit during chaos test", event.id),
                    );
                    return Ok(None);
                }
                Some(EventFault::Duplicate) => {
                    self.record_seen(event.id.clone());
                    self.processed_count += 1;
                    ledger.record_event(
                        AuditEventType::CommandAllowed,
                        ActionSeverity::Routine,
                        format!("Event '{}' processed under duplicate fault injection", event.id),
                    );
                    return Ok(Some(event));
                }
                Some(EventFault::CorruptedPayload) => {
                    self.rejected_count += 1;
                    let err = EventError::CorruptedPayload("Packet payload checksum failure or corrupted JSON".to_string());
                    ledger.record_event(
                        AuditEventType::CommandException,
                        ActionSeverity::HighImpact,
                        format!("Event '{}' rejected due to corrupted payload: {}", event.id, err),
                    );
                    return Err(err);
                }
                Some(fault) => {
                    if attempt <= self.max_retries {
                        self.recovered_count += 1;
                    } else {
                        let err = fault.to_error();
                        ledger.record_event(
                            AuditEventType::CommandException,
                            ActionSeverity::HighImpact,
                            format!("Event '{}' failed after {} retries: {}", event.id, self.max_retries, err),
                        );
                        return Err(err);
                    }
                }
                None => {
                    self.record_seen(event.id.clone());
                    self.processed_count += 1;
                    ledger.record_event(
                        AuditEventType::CommandAllowed,
                        ActionSeverity::Routine,
                        format!("Event '{}' processed successfully", event.id),
                    );
                    return Ok(Some(event));
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EventBenchmarkStats {
    pub total_requests: usize,
    pub successful_processed: usize,
    pub p50_latency_us: f64,
    pub p90_latency_us: f64,
    pub p99_latency_us: f64,
    pub max_latency_us: f64,
    pub total_duration: Duration,
    pub throughput_ops_per_sec: f64,
    pub memory_allocated_bytes: usize,
}

pub struct EventBenchmarkHarness;

impl EventBenchmarkHarness {
    fn compute_stats(mut latencies: Vec<f64>, total_requests: usize, successful: usize, duration: Duration, mem: usize) -> EventBenchmarkStats {
        latencies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let p50 = Self::percentile(&latencies, 0.50);
        let p90 = Self::percentile(&latencies, 0.90);
        let p99 = Self::percentile(&latencies, 0.99);
        let max = latencies.last().copied().unwrap_or(0.0);
        let secs = duration.as_secs_f64();
        let throughput = if secs > 0.0 { total_requests as f64 / secs } else { 0.0 };
        EventBenchmarkStats {
            total_requests,
            successful_processed: successful,
            p50_latency_us: p50,
            p90_latency_us: p90,
            p99_latency_us: p99,
            max_latency_us: max,
            total_duration: duration,
            throughput_ops_per_sec: throughput,
            memory_allocated_bytes: mem,
        }
    }

    pub fn run_benchmark(processor: &mut ResilientEventProcessor, num_requests: usize) -> EventBenchmarkStats {
        let sender = AgentId::new("bench-sender");
        let receiver = AgentId::new("bench-receiver");
        let mut latencies_us = Vec::with_capacity(num_requests);
        let start = Instant::now();
        let mut successful = 0;

        for i in 0..num_requests {
            let event = InterAgentEvent::new(sender.clone(), Some(receiver.clone()), "benchmark.event", serde_json::json!({ "sequence": i }));
            let t0 = Instant::now();
            if let Ok(Some(_)) = processor.process_event(event) { successful += 1; }
            latencies_us.push(t0.elapsed().as_secs_f64() * 1_000_000.0);
        }

        Self::compute_stats(latencies_us, num_requests, successful, start.elapsed(), processor.memory_usage_bytes())
    }

    pub fn run_concurrent_benchmark(num_requests: usize, concurrency: usize) -> EventBenchmarkStats {
        const NUM_SHARDS: usize = 16;
        let per_shard_cap = (num_requests / NUM_SHARDS) + 64;
        let shards: Arc<Vec<Mutex<ResilientEventProcessor>>> = Arc::new(
            (0..NUM_SHARDS).map(|_| Mutex::new(ResilientEventProcessor::new(EventFaultPolicy::none(), 3).with_max_capacity(per_shard_cap))).collect(),
        );

        let start = Instant::now();
        let per_thread = num_requests / concurrency;
        let mut handles = Vec::with_capacity(concurrency);

        for t_idx in 0..concurrency {
            let shards_clone = Arc::clone(&shards);
            handles.push(std::thread::spawn(move || {
                let sender = AgentId::new(&format!("worker-{}", t_idx));
                let receiver = AgentId::new("drain-target");
                let mut local_latencies = Vec::with_capacity(per_thread);
                let mut local_success = 0;
                for i in 0..per_thread {
                    let event = InterAgentEvent::new(sender.clone(), Some(receiver.clone()), "concurrent.event", serde_json::json!({ "thread": t_idx, "idx": i }));
                    let mut hasher = DefaultHasher::new();
                    event.id.hash(&mut hasher);
                    let shard_idx = (hasher.finish() as usize) % NUM_SHARDS;
                    let t0 = Instant::now();
                    let res = { shards_clone[shard_idx].lock().expect("lock acquired").process_event(event) };
                    if let Ok(Some(_)) = res { local_success += 1; }
                    local_latencies.push(t0.elapsed().as_secs_f64() * 1_000_000.0);
                }
                (local_latencies, local_success)
            }));
        }

        let mut all_latencies = Vec::with_capacity(num_requests);
        let mut total_success = 0;
        for handle in handles {
            if let Ok((latencies, success)) = handle.join() {
                all_latencies.extend(latencies);
                total_success += success;
            }
        }

        let mem_usage: usize = shards.iter().map(|s| s.lock().expect("lock acquired").memory_usage_bytes()).sum();
        Self::compute_stats(all_latencies, num_requests, total_success, start.elapsed(), mem_usage)
    }

    fn percentile(sorted: &[f64], pct: f64) -> f64 {
        if sorted.is_empty() { return 0.0; }
        let idx = ((sorted.len() as f64) * pct).floor() as usize;
        sorted[idx.min(sorted.len() - 1)]
    }
}

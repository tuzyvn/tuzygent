use std::collections::{HashSet, VecDeque};

mod audit;
mod processing;
mod state;

#[derive(Debug, Clone)]
pub struct ResilientEventProcessor {
    policy: super::fault::EventFaultPolicy,
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
}

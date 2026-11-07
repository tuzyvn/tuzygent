use crate::cli::fault::CliFaultPolicy;
use crate::traits::EngineOutput;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};

mod configuration;
mod metrics;
mod response_cache;

/// Wrapper tự phục hồi cho ComputeEngine CLI với cơ chế retry có kiểm soát, bounded memory cache và injection fault testing
#[derive(Clone)]
pub struct ResilientCliEngine<E> {
    pub(super) inner: E,
    pub(super) fault_policy: Arc<Mutex<CliFaultPolicy>>,
    pub(super) max_retries: usize,
    pub(super) retry_delay_ms: u64,
    pub(super) resolved_count: Arc<AtomicUsize>,
    pub(super) recovered_count: Arc<AtomicUsize>,
    pub(super) rejected_count: Arc<AtomicUsize>,
    pub(super) response_cache: Arc<Mutex<HashMap<String, EngineOutput>>>,
    pub(super) cache_queue: Arc<Mutex<VecDeque<String>>>,
    pub(super) max_capacity: Option<usize>,
    pub(super) backoff_policy: Option<crate::backoff::BackoffPolicy>,
}

pub type ResilientCliManager<E> = ResilientCliEngine<E>;

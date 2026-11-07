use crate::api::fault::ApiFaultPolicy;
use crate::traits::EngineOutput;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};

mod cache;
mod configuration;
mod execution;
mod metrics;

/// Wrapper tự phục hồi cho ComputeEngine với cơ chế retry có kiểm soát, bounded memory cache và injection fault testing
#[derive(Clone)]
pub struct ResilientApiEngine<E> {
    pub(super) inner: E,
    pub(super) fault_policy: Arc<Mutex<ApiFaultPolicy>>,
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

pub type ResilientApiWrapper<E> = ResilientApiEngine<E>;
pub type ResilientApiManager<E> = ResilientApiEngine<E>;

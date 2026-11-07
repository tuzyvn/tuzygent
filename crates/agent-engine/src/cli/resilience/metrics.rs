use super::ResilientCliEngine;
use std::sync::atomic::Ordering;

impl<E> ResilientCliEngine<E> {
    pub fn resolved_count(&self) -> usize {
        self.resolved_count.load(Ordering::SeqCst)
    }

    pub fn recovered_count(&self) -> usize {
        self.recovered_count.load(Ordering::SeqCst)
    }

    pub fn rejected_count(&self) -> usize {
        self.rejected_count.load(Ordering::SeqCst)
    }

    pub fn reset_metrics(&self) {
        self.resolved_count.store(0, Ordering::SeqCst);
        self.recovered_count.store(0, Ordering::SeqCst);
        self.rejected_count.store(0, Ordering::SeqCst);
    }
}

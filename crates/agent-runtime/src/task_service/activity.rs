use super::TaskService;
use agent_domain::TaskId;
use std::sync::atomic::Ordering;

impl TaskService {
    pub async fn active_task_count(&self) -> usize {
        match self.cancellation_flags.read() {
            Ok(lock) => lock.len(),
            Err(poisoned) => poisoned.into_inner().len(),
        }
    }

    pub async fn is_active(&self, id: &TaskId) -> bool {
        match self.cancellation_flags.read() {
            Ok(lock) => lock.contains_key(&id.0),
            Err(poisoned) => poisoned.into_inner().contains_key(&id.0),
        }
    }

    pub async fn active_task_ids(&self) -> Vec<TaskId> {
        let flags = match self.cancellation_flags.read() {
            Ok(lock) => lock,
            Err(poisoned) => poisoned.into_inner(),
        };
        flags.keys().map(|id| TaskId(id.clone())).collect()
    }

    pub(super) fn trigger_all_cancellation_flags(&self) {
        let flags = match self.cancellation_flags.read() {
            Ok(lock) => lock,
            Err(poisoned) => poisoned.into_inner(),
        };
        for flag in flags.values() {
            flag.store(true, Ordering::SeqCst);
        }
    }

    pub(super) fn trigger_cancellation_flag(&self, id: &TaskId) {
        let flags = match self.cancellation_flags.read() {
            Ok(lock) => lock,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(flag) = flags.get(&id.0) {
            flag.store(true, Ordering::SeqCst);
        }
    }
}

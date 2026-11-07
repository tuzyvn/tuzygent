use crate::durable::fault::{DurableMemoryError, DurableMemoryFault};
use crate::durable::resilience::ResilientDurableMemoryStore;

impl ResilientDurableMemoryStore {
    pub(crate) fn check_fault_injection(
        &mut self,
        attempt: usize,
    ) -> Result<Option<()>, DurableMemoryError> {
        if let Some(fault) = self.fault_policy.should_inject() {
            match fault {
                DurableMemoryFault::SocketTimeout
                | DurableMemoryFault::ConnectionReset
                | DurableMemoryFault::TransientIo
                | DurableMemoryFault::LockFailure => {
                    if attempt <= self.max_retries {
                        self.recovered_count += 1;
                        Ok(Some(()))
                    } else {
                        Err(fault.to_error())
                    }
                }
                DurableMemoryFault::CorruptedState => {
                    self.rejected_count += 1;
                    Err(fault.to_error())
                }
            }
        } else {
            Ok(None)
        }
    }

    pub(crate) fn handle_transient_error(
        &mut self,
        attempt: usize,
        err: std::io::Error,
    ) -> Result<(), DurableMemoryError> {
        if attempt <= self.max_retries {
            self.recovered_count += 1;
            Ok(())
        } else {
            Err(DurableMemoryError::from(err))
        }
    }
}

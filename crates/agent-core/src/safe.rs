use crate::fault::CoreExecutionError;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// Extension trait providing zero-panic safe extraction for `Option<T>` and `Result<T, E>`.
/// This trait eliminates unhandled panics and unwrap-induced crashes across embedded AgentCore instances.
pub trait SafeUnwrapExt<T> {
    /// Safely unpacks the value, returning `fallback` if None or Err.
    fn safe_unwrap_or(self, fallback: T) -> T;

    /// Safely unpacks the value or computes a fallback using `op`.
    fn safe_unwrap_or_else<F: FnOnce() -> T>(self, op: F) -> T;

    /// Safely unpacks the value or returns the type's `Default::default()`.
    fn safe_unwrap_or_default(self) -> T
    where
        T: Default;

    /// Converts into a structured `Result<T, CoreExecutionError>`, completely preventing panic.
    fn safe_resolve(self, context: &str) -> Result<T, CoreExecutionError>;
}

impl<T> SafeUnwrapExt<T> for Option<T> {
    fn safe_unwrap_or(self, fallback: T) -> T {
        match self {
            Some(v) => v,
            None => fallback,
        }
    }

    fn safe_unwrap_or_else<F: FnOnce() -> T>(self, op: F) -> T {
        match self {
            Some(v) => v,
            None => op(),
        }
    }

    fn safe_unwrap_or_default(self) -> T
    where
        T: Default,
    {
        self.unwrap_or_default()
    }

    fn safe_resolve(self, context: &str) -> Result<T, CoreExecutionError> {
        match self {
            Some(v) => Ok(v),
            None => Err(CoreExecutionError::UnwrapSafetyViolation(format!(
                "Unwrap avoided on None value in context: {}",
                context
            ))),
        }
    }
}

impl<T, E: std::fmt::Display> SafeUnwrapExt<T> for Result<T, E> {
    fn safe_unwrap_or(self, fallback: T) -> T {
        match self {
            Ok(v) => v,
            Err(_) => fallback,
        }
    }

    fn safe_unwrap_or_else<F: FnOnce() -> T>(self, op: F) -> T {
        match self {
            Ok(v) => v,
            Err(_) => op(),
        }
    }

    fn safe_unwrap_or_default(self) -> T
    where
        T: Default,
    {
        self.unwrap_or_default()
    }

    fn safe_resolve(self, context: &str) -> Result<T, CoreExecutionError> {
        match self {
            Ok(v) => Ok(v),
            Err(e) => Err(CoreExecutionError::UnwrapSafetyViolation(format!(
                "Unwrap avoided on Err ('{}') in context: {}",
                e, context
            ))),
        }
    }
}

/// Panic isolation helper for closures and agent sub-tasks.
pub struct SafeGuard;

impl SafeGuard {
    /// Runs a closure with panic catching. If a panic occurs, returns the provided fallback.
    pub fn catch_unwind_safe<F, R>(f: F, fallback: R) -> R
    where
        F: FnOnce() -> R,
    {
        match catch_unwind(AssertUnwindSafe(f)) {
            Ok(val) => val,
            Err(_) => fallback,
        }
    }

    /// Runs a closure with panic catching, converting any caught panic to `CoreExecutionError::Panic`.
    pub fn catch_unwind_result<F, R>(f: F) -> Result<R, CoreExecutionError>
    where
        F: FnOnce() -> R,
    {
        match catch_unwind(AssertUnwindSafe(f)) {
            Ok(val) => Ok(val),
            Err(payload) => {
                let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = payload.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "Unknown panic payload".to_string()
                };
                Err(CoreExecutionError::Panic(msg))
            }
        }
    }
}

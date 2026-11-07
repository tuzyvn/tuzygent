mod cache;
mod config;
mod core;
mod recovery;

pub use self::core::{ResilientDurableMemory, ResilientDurableMemoryStore};

#[cfg(test)]
mod tests;

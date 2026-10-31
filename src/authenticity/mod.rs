//! Model authenticity checks (quality monitoring v3): does a channel return
//! the model it claims? See docs/model-authenticity-design.md.
pub mod baseline;
pub mod bundle;
pub mod deep;
pub mod defaults;
pub mod engine;
pub mod evaluate;
pub mod fingerprint;
pub mod public;
pub mod schedule;

#[cfg(test)]
mod db_tests;

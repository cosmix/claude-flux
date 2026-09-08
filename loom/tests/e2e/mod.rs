//! End-to-end test infrastructure for loom
//!
//! This module provides test utilities and fixtures for integration testing
//! the loom orchestration system.

pub mod context_variables;
pub mod criteria_validation;
pub mod daemon_config;
pub mod fixtures;
pub mod handoff;
pub mod helpers;
pub mod manual_mode;
pub mod merge;
pub mod parallel;
pub mod sequential;
pub mod sessions;
pub mod tmux_backend;
pub mod tmux_reconcile;
pub mod uncommitted_changes;
pub mod web_terminal;

pub use fixtures::*;
pub use helpers::*;

//! Integration tests for loom orchestration features
//!
//! These tests verify end-to-end behavior of loom's orchestration features
//! including dependency inheritance, worktree management, and conflict resolution.

pub mod capsule;
pub mod context_catalog;
pub mod dependency_cleanup;
pub mod dependency_conflict;
pub mod dependency_multi;
pub mod dependency_simple;
pub mod helpers;
pub mod hooks_commit_filter;
pub mod hooks_git_add_guard;
pub mod hooks_no_preexisting_failures;
pub mod hooks_poll_guard;
pub mod hooks_read_guard;
pub mod hooks_skill_project;
pub mod hooks_skill_trigger;
pub mod hooks_spawn_guard;
pub mod hooks_subagent_verify_guard;
pub mod implementer_defaults;
pub mod install_assets;
pub mod merge_conflict_recovery;
pub mod plan_verify;
pub mod source_graph_fixtures;
pub mod update_notice;

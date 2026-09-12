//! Execution loop; final results are a snapshot, never a history of transient failures.
use super::{event_handler::EventHandler, recovery::Recovery, stage_executor::StageExecutor};
use super::{Orchestrator, OrchestratorResult};
use crate::fs::work_integrity::validate_work_dir_state;
use crate::orchestrator::tick;
use crate::utils::{cleanup_terminal, install_terminal_panic_hook};
use anyhow::{Context, Result};
use chrono::Utc;
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

impl Orchestrator {
    /// Execute until completion, shutdown, or the selected mode's exit condition.
    pub fn run(&mut self) -> Result<OrchestratorResult> {
        install_terminal_panic_hook();
        let started_at = Utc::now();
        let mut spawned = self.initialize_run()?;
        let mut printed = false;
        let mut status_update = Instant::now();
        loop {
            if self.shutdown_requested() {
                break;
            }
            spawned += self.run_tick(&mut printed)?;
            if !self.config.manual_mode {
                self.poll_run_events(&mut status_update)?;
            }
            if self.run_finished() {
                break;
            }
            self.wait_for_next_tick();
        }
        // Clear liveness even when the final snapshot cannot be read.
        tick::clear(&self.config.work_dir);
        crate::orchestrator::scheduling_report::clear(&self.config.work_dir);
        cleanup_terminal();
        super::run_result::collect_result(&self.config.work_dir, &self.graph, spawned, started_at)
    }

    // Reconcile before syncing and before orphan recovery: recovery removes merge attribution.
    fn initialize_run(&mut self) -> Result<usize> {
        validate_work_dir_state(&self.config.repo_root)
            .context("Work directory integrity check failed")?;
        self.reconcile_and_update_graph()
            .context("Failed to reconcile active main-repo merge")?;
        self.sync_graph_with_stage_files()
            .context("Failed to sync graph with existing stage files")?;
        let recovered = self
            .recover_orphaned_sessions()
            .context("Failed to recover orphaned sessions")?;
        if recovered > 0 {
            println!("Recovered {recovered} orphaned session(s) - stages reset to Ready");
        }
        self.graph.refresh_ready_status();
        self.sync_queued_status_to_files()
            .context("Failed to sync queued status to files")?;
        self.check_pending_disputes()
            .context("Failed to check pending disputes")?;
        self.apply_pending_verdicts()
            .context("Failed to apply pending verdicts")?;
        self.spawn_merge_resolution_sessions()
            .context("Failed to spawn merge resolution sessions")
    }

    // Reconcile before sync on every tick; spools drain even in manual mode.
    fn run_tick(&mut self, printed_view_instructions: &mut bool) -> Result<usize> {
        tick::record(&self.config.work_dir, tick::Phase::Sync);
        self.reconcile_and_update_graph()
            .context("Failed to reconcile active main-repo merge")?;
        self.sync_graph_with_stage_files()
            .context("Failed to sync graph with stage files")?;
        self.sync_queued_status_to_files()
            .context("Failed to sync queued status to files")?;
        self.check_pending_disputes()
            .context("Failed to check pending disputes")?;
        self.apply_pending_verdicts()
            .context("Failed to apply pending verdicts")?;
        let merge_sessions_spawned = self
            .spawn_merge_resolution_sessions()
            .context("Failed to spawn merge resolution sessions")?;
        self.drain_stage_spools();
        tick::record(&self.config.work_dir, tick::Phase::Spawning);
        let started = self
            .start_ready_stages()
            .context("Failed to start ready stages")?;
        if started > 0 && !*printed_view_instructions && !self.config.manual_mode {
            *printed_view_instructions = true;
            println!();
            println!("Sessions are now running. To view progress:");
            println!("  loom status               View overall progress");
            println!();
        }
        Ok(merge_sessions_spawned + started)
    }

    fn poll_run_events(&mut self, last_status_update: &mut Instant) -> Result<()> {
        tick::record(&self.config.work_dir, tick::Phase::Events);
        let events = self
            .monitor
            .poll()
            .context("Failed to poll monitor for events")?;
        self.handle_events(events)
            .context("Failed to handle monitor events")?;
        if last_status_update.elapsed() >= self.config.status_update_interval {
            self.print_status_update();
            *last_status_update = Instant::now();
        }
        Ok(())
    }

    fn run_finished(&self) -> bool {
        if self.config.manual_mode {
            return true;
        }
        if self.config.watch_mode {
            return self.all_stages_terminal();
        }
        // Failures keep normal mode alive for operator recovery.
        self.graph.is_complete()
    }

    fn shutdown_requested(&self) -> bool {
        self.config
            .shutdown_flag
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
    }

    fn wait_for_next_tick(&self) {
        tick::record(&self.config.work_dir, tick::Phase::Idle);
        let started = Instant::now();
        while started.elapsed() < self.config.poll_interval && !self.shutdown_requested() {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

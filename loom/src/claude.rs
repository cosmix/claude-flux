//! Shared Claude binary resolution utilities.

use anyhow::{bail, Result};
use std::path::PathBuf;

/// Claude model aliases `loom pressure` accepts for its foreground steps,
/// cheapest tier first (mirrors hooks/spawn-guard.sh's tier ranking).
pub const CLAUDE_MODELS: &[&str] = &["haiku", "sonnet", "opus", "fable"];

/// Claude model both pressure-run steps default to.
pub const DEFAULT_PRESSURE_CLAUDE_MODEL: &str = "opus";

/// Find the absolute path to the claude binary
///
/// On macOS, spawned terminals don't inherit the parent's PATH, so we need
/// to resolve claude's path at script generation time.
pub fn find_claude_path() -> Result<PathBuf> {
    // First try which::which (uses current PATH)
    if let Ok(path) = which::which("claude") {
        return Ok(path);
    }

    // Common installation locations
    // Note: ~/.claude/local/claude is the official Claude Code install location
    let candidates = [
        dirs::home_dir().map(|h| h.join(".claude/local/claude")),
        dirs::home_dir().map(|h| h.join(".local/bin/claude")),
        dirs::home_dir().map(|h| h.join(".cargo/bin/claude")),
        Some(PathBuf::from("/usr/local/bin/claude")),
        Some(PathBuf::from("/opt/homebrew/bin/claude")),
    ];

    for candidate in candidates.into_iter().flatten() {
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    bail!("claude binary not found in PATH or common locations. Checked: ~/.claude/local/claude, ~/.local/bin/claude, /usr/local/bin/claude")
}

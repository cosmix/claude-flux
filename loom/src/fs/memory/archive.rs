//! Durable run-state archiving before the transient work directory is removed.

use anyhow::{Context, Result};
use chrono::Utc;
use std::fs;
use std::path::{Path, PathBuf};

use crate::context::delivery::plan_key_from;

/// Copy `<work_dir>/memory/` and `<work_dir>/telemetry/` (when present) to
/// `<main_root>/.loom/memory/archive/<plan-id-or-default>-<YYYYmmddTHHMMSSZ>/`.
///
/// Returns the archive path, or `None` when there was nothing to archive or
/// archiving failed. Failures are deliberately contained here so cleanup and
/// plan completion can continue.
pub fn archive_run_state(
    work_dir: &Path,
    main_root: &Path,
    plan_id: Option<&str>,
) -> Option<PathBuf> {
    match try_archive_run_state(work_dir, main_root, plan_id) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("Warning: Failed to archive run state: {error}");
            None
        }
    }
}

fn try_archive_run_state(
    work_dir: &Path,
    main_root: &Path,
    plan_id: Option<&str>,
) -> Result<Option<PathBuf>> {
    let source_names: Vec<_> = ["memory", "telemetry"]
        .into_iter()
        .filter(|name| work_dir.join(name).exists())
        .collect();
    if source_names.is_empty() {
        return Ok(None);
    }

    let archive_name = format!(
        "{}-{}",
        plan_key_from(plan_id),
        Utc::now().format("%Y%m%dT%H%M%SZ")
    );
    let archive_path = main_root.join(".loom/memory/archive").join(archive_name);
    fs::create_dir_all(&archive_path)
        .with_context(|| format!("creating archive directory {}", archive_path.display()))?;

    for name in source_names {
        copy_path(&work_dir.join(name), &archive_path.join(name))?;
    }

    Ok(Some(archive_path))
}

fn copy_path(source: &Path, destination: &Path) -> Result<()> {
    if source.is_dir() {
        fs::create_dir_all(destination)
            .with_context(|| format!("creating archive directory {}", destination.display()))?;
        for entry in fs::read_dir(source)
            .with_context(|| format!("reading archive source {}", source.display()))?
        {
            let entry =
                entry.with_context(|| format!("reading archive source {}", source.display()))?;
            copy_path(&entry.path(), &destination.join(entry.file_name()))?;
        }
        return Ok(());
    }

    fs::copy(source, destination).with_context(|| {
        format!(
            "copying archive source {} to {}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(())
}

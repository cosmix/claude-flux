use crate::claude::find_claude_path;
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
};

/// Summarize a plan file by passing its content to Claude Haiku.
///
/// Falls back to the plan's first paragraph if claude is unavailable.
pub(super) fn summarize_plan(plan_path: &Path) -> String {
    let content = match fs::read_to_string(plan_path) {
        Ok(c) => c,
        Err(_) => return "No plan description available.".to_string(),
    };

    let claude_path = match find_claude_path() {
        Ok(p) => p,
        Err(_) => return fallback_description(&content),
    };

    let prompt = "Summarize this execution plan in 2-4 concise sentences. \
                  Focus on what the plan accomplishes, not its structure. \
                  Output only the summary, no preamble.";

    let result = Command::new(&claude_path)
        .args(["-p", "--model", "haiku", prompt])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            if let Some(ref mut stdin) = child.stdin {
                let _ = stdin.write_all(content.as_bytes());
            }
            child.wait_with_output()
        });

    match result {
        Ok(output) if output.status.success() => {
            let summary = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if summary.is_empty() {
                fallback_description(&content)
            } else {
                summary
            }
        }
        _ => fallback_description(&content),
    }
}

/// Extract the first paragraph after the heading as a fallback description.
pub(super) fn fallback_description(content: &str) -> String {
    let metadata_marker = "<!-- loom METADATA -->";
    let body = if let Some(idx) = content.find(metadata_marker) {
        &content[..idx]
    } else {
        content
    };

    let mut lines = body.lines().peekable();
    // Skip to first heading
    while let Some(line) = lines.peek() {
        if line.trim_start().starts_with('#') {
            lines.next();
            break;
        }
        lines.next();
    }
    // Skip blank lines after heading
    while let Some(line) = lines.peek() {
        if line.trim().is_empty() {
            lines.next();
        } else {
            break;
        }
    }
    // Collect until next blank line (first paragraph only)
    let mut paragraph = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            break;
        }
        paragraph.push(line);
    }

    let trimmed = paragraph.join("\n").trim().to_string();
    if trimmed.is_empty() {
        "No plan description available.".to_string()
    } else {
        trimmed
    }
}

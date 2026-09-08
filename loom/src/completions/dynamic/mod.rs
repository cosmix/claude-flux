//! Dynamic shell completions for loom CLI.
//!
//! This module provides context-aware tab-completion for commands,
//! subcommands, flags, plan files, stage IDs, session IDs, knowledge
//! files, and memory entry types.

mod commands;
mod knowledge;
mod memory;
mod plans;
mod sessions;
mod stages;

#[cfg(test)]
mod tests;

use anyhow::Result;
use clap::{Arg, Command, CommandFactory};
use std::path::Path;

pub use commands::{
    complete_codex_model_names, complete_commands, complete_flags, complete_model_names,
    complete_shell_types, complete_subcommands, complete_trigger_types,
};
pub use knowledge::{complete_knowledge_files, complete_knowledge_scopes};
pub use memory::complete_memory_entry_types;
pub use plans::complete_plan_files;
pub use sessions::{complete_session_ids, complete_stage_or_session_ids};
pub use stages::{complete_stage_ids, complete_stage_ids_filtered};

/// Context for shell completion
#[derive(Debug, Clone)]
pub struct CompletionContext {
    pub cwd: String,
    pub shell: String,
    pub cmdline: String,
    pub current_word: String,
    pub prev_word: String,
}

impl CompletionContext {
    /// Parse completion context from shell-provided arguments
    ///
    /// # Arguments
    ///
    /// * `shell` - Shell type (bash, zsh, fish)
    /// * `args` - Arguments passed from shell completion system
    pub fn from_args(shell: &str, args: &[String]) -> Self {
        let cwd = args.first().cloned().unwrap_or_else(|| ".".to_string());
        let cmdline = args.get(1).cloned().unwrap_or_default();
        let current_word = args.get(2).cloned().unwrap_or_default();
        let prev_word = args.get(3).cloned().unwrap_or_default();

        Self {
            cwd,
            shell: shell.to_string(),
            cmdline,
            current_word,
            prev_word,
        }
    }
}

/// Parse the command line into words, stripping the leading "loom" if present.
fn parse_cmdline_words(cmdline: &str) -> Vec<String> {
    let words = shell_words::split(cmdline)
        .unwrap_or_else(|_| cmdline.split_whitespace().map(String::from).collect());
    if words.first().map(|w| w.as_str()) == Some("loom") {
        words[1..].to_vec()
    } else {
        words
    }
}

fn argument_for_long<'a>(command: &'a Command, name: &str) -> Option<&'a Arg> {
    command
        .get_arguments()
        .find(|argument| argument.get_long() == Some(name))
}

fn argument_for_short(command: &Command, name: char) -> Option<&Arg> {
    command
        .get_arguments()
        .find(|argument| argument.get_short() == Some(name))
}

fn option_consumes_next(argument: Option<&Arg>) -> bool {
    argument.is_some_and(|argument| argument.get_action().takes_values())
}

fn matching_subcommand<'a>(command: &'a Command, word: &str) -> Option<&'a Command> {
    command.get_subcommands().find(|subcommand| {
        subcommand.get_name() == word || subcommand.get_all_aliases().any(|alias| alias == word)
    })
}

/// Extract only actual Clap subcommands, skipping positional arguments and
/// values consumed by options. The current partial word is excluded once, at
/// the end, instead of removing every equal token.
fn extract_command_path(words: &[String], current_word: &str) -> Vec<String> {
    let mut complete_words = words;
    if !current_word.is_empty() && words.last().map(String::as_str) == Some(current_word) {
        complete_words = &words[..words.len() - 1];
    }

    let root = crate::cli::Cli::command();
    let mut command = &root;
    let mut path = Vec::new();
    let mut consume_value = false;

    for word in complete_words {
        if consume_value {
            consume_value = false;
            continue;
        }
        if word == "--" {
            break;
        }
        if let Some(long) = word.strip_prefix("--") {
            let (name, inline_value) = long
                .split_once('=')
                .map_or((long, false), |(name, _)| (name, true));
            consume_value = !inline_value && option_consumes_next(argument_for_long(command, name));
            continue;
        }
        if let Some(shorts) = word.strip_prefix('-').filter(|shorts| !shorts.is_empty()) {
            if let Some(short) = shorts.chars().next() {
                consume_value = shorts.len() == short.len_utf8()
                    && option_consumes_next(argument_for_short(command, short));
            }
            continue;
        }
        if let Some(subcommand) = matching_subcommand(command, word) {
            path.push(subcommand.get_name().to_string());
            command = subcommand;
        }
    }

    path
}

/// Main entry point for dynamic completions.
///
/// Determines what to complete based on context and prints results to stdout.
pub fn complete_dynamic(ctx: &CompletionContext) -> Result<()> {
    let cwd = Path::new(&ctx.cwd);
    let prefix = &ctx.current_word;
    let words = parse_cmdline_words(&ctx.cmdline);
    let command_path = extract_command_path(&words, prefix);
    let cmd_path: Vec<&str> = command_path.iter().map(String::as_str).collect();

    let completions = route_completion(cwd, prefix, &ctx.prev_word, &cmd_path, &ctx.cmdline)?;

    for completion in completions {
        println!("{completion}");
    }

    Ok(())
}

/// Route to the appropriate completion function based on context.
fn route_completion(
    cwd: &Path,
    prefix: &str,
    prev_word: &str,
    cmd_path: &[&str],
    cmdline: &str,
) -> Result<Vec<String>> {
    // If current word starts with -, complete flags based on command path
    if prefix.starts_with('-') {
        return complete_flags(cmd_path, prefix);
    }

    // Check if prev_word is a flag that expects a value
    if let Some(completions) = complete_flag_value(cwd, prefix, prev_word, cmd_path, cmdline)? {
        return Ok(completions);
    }

    // Route based on command path depth
    match cmd_path {
        [] => complete_commands(prefix),
        [cmd] => complete_after_command(cwd, prefix, cmd),
        [cmd, sub] => complete_after_subcommand(cwd, prefix, cmd, sub),
        [_cmd, "output", sub] => complete_after_subcommand(cwd, prefix, "output", sub),
        _ => Ok(Vec::new()),
    }
}

/// Complete values after a flag that expects an argument.
fn complete_flag_value(
    cwd: &Path,
    prefix: &str,
    prev_word: &str,
    _cmd_path: &[&str],
    cmdline: &str,
) -> Result<Option<Vec<String>>> {
    match prev_word {
        "--stage" => {
            let results = complete_stage_ids(cwd, prefix)?;
            Ok(Some(results))
        }
        "--entry-type" | "-t" if cmdline.contains("memory") => {
            let results = complete_memory_entry_types(prefix)?;
            Ok(Some(results))
        }
        "--model" | "--claude-model" | "--address-model" => {
            let results = complete_model_names(prefix)?;
            Ok(Some(results))
        }
        "--codex-model" => {
            let results = complete_codex_model_names(prefix)?;
            Ok(Some(results))
        }
        "--trigger" => {
            let results = complete_trigger_types(prefix)?;
            Ok(Some(results))
        }
        "--scope" => {
            let results = complete_knowledge_scopes(prefix)?;
            Ok(Some(results))
        }
        "--session" => {
            let results = complete_session_ids(cwd, prefix)?;
            Ok(Some(results))
        }
        // --tail expects a number; suppress stage-ID suggestions
        "--tail" => Ok(Some(Vec::new())),
        _ => Ok(None),
    }
}

/// Complete after a single top-level command.
fn complete_after_command(cwd: &Path, prefix: &str, cmd: &str) -> Result<Vec<String>> {
    if commands::has_subcommands(cmd) {
        return complete_subcommands(cmd, prefix);
    }

    match cmd {
        // init and pressure take a file path — return empty so the shell
        // falls back to native path completion
        "init" | "pressure" => Ok(Vec::new()),
        "completions" => complete_shell_types(prefix),
        "attach" | "check" | "diagnose" | "resume" => complete_stage_ids(cwd, prefix),
        _ => Ok(Vec::new()),
    }
}

/// Complete after a subcommand (two-level depth).
fn complete_after_subcommand(
    cwd: &Path,
    prefix: &str,
    cmd: &str,
    sub: &str,
) -> Result<Vec<String>> {
    match (cmd, sub) {
        // Stage subcommands that take stage IDs
        ("stage", "complete") => complete_stage_ids_filtered(cwd, prefix, &["executing"]),
        ("stage", "retry") => complete_stage_ids_filtered(
            cwd,
            prefix,
            &[
                "blocked",
                "completed-with-failures",
                "merge-blocked",
                "needs-handoff",
            ],
        ),
        ("stage", "merge") => {
            complete_stage_ids_filtered(cwd, prefix, &["merge-conflict", "merge-blocked"])
        }
        ("stage", "reset") => complete_stage_ids_filtered(
            cwd,
            prefix,
            &[
                "blocked",
                "executing",
                "completed-with-failures",
                "needs-handoff",
            ],
        ),
        ("stage", "human-review" | "dispute-criteria") => complete_stage_ids(cwd, prefix),
        ("stage", "block" | "hold" | "release" | "skip" | "waiting" | "resume") => {
            complete_stage_ids(cwd, prefix)
        }
        ("stage", "output") => complete_subcommands("output", prefix),

        // Output subcommands take stage IDs
        ("output", "set" | "get" | "list" | "remove") => complete_stage_ids(cwd, prefix),

        // Session subcommands
        ("sessions", "kill") => complete_session_ids(cwd, prefix),

        // Worktree subcommands
        ("worktree", "remove") => complete_stage_ids(cwd, prefix),

        // Knowledge subcommands
        ("knowledge", "update" | "replace-section") => complete_knowledge_files(prefix),

        // Plan subcommands
        ("plan", "verify") => complete_plan_files(cwd, prefix),

        _ => Ok(Vec::new()),
    }
}

use crate::commands::{
    attach, clean, config, context, diagnose, graph, handoff, hook, init, install_assets,
    knowledge, map, memory, plan, pressure, repair, resume, review, run, self_update, sessions,
    skill_index, stage, status, stop, subagents, usage, verify, worktree_cmd,
};
use crate::completions::{complete_dynamic, generate_completions, CompletionContext, Shell};
use anyhow::Result;
use std::path::PathBuf;
use std::str::FromStr;

use super::dispatch_stage;
use super::types::{
    Commands, ContextCommands, HookCommands, KnowledgeCommands, MemoryCommands, PlanCommands,
    SessionsCommands, WorktreeCommands,
};

/// The admin proof a `loom stage complete` invocation needs, if any.
///
/// An unprivileged completion needs none. A privileged one authorizes itself
/// through `admin_proof::authorize`, which uses a broker's `LOOM_ADMIN_PROOF`
/// when one is present and otherwise mints from the daemon token the operator
/// can already read. No flag, and nothing for a human to carry between
/// commands.
pub(super) fn resolve_completion_proof(
    stage_id: &str,
    no_verify: bool,
    force_unsafe: bool,
    assume_merged: bool,
) -> anyhow::Result<Option<String>> {
    if !(no_verify || force_unsafe || assume_merged) {
        return Ok(None);
    }
    // Resolved rather than a hardcoded `.work` literal: only `WorkDir::new`
    // decides whether this project uses the nested or the legacy layout.
    let Ok(work_dir) = crate::fs::work_dir::WorkDir::new(".") else {
        return Ok(None);
    };
    stage::admin_proof::authorize(
        work_dir.root(),
        stage::admin_proof::AdminProofRequest::completion(
            stage_id,
            no_verify,
            force_unsafe,
            assume_merged,
        ),
    )
}

/// `loom stage admin-proof` — mint one capability and print it, nothing else.
///
/// The secret arrives in `LOOM_ADMIN_TOKEN` and is never read from disk here,
/// so a caller that can invoke loom but cannot read `.loom/work/admin.token` gains
/// nothing: a wrong secret simply mints a proof that verification rejects.
/// That is what separates this command from `admin_proof::authorize`, which
/// reads the token and therefore relies on the sandbox to keep an agent out.
pub(super) fn print_minted_proof(
    stage_id: Option<String>,
    daemon_stop: bool,
    no_verify: bool,
    force_unsafe: bool,
    assume_merged: bool,
) -> Result<()> {
    if daemon_stop {
        println!("{}", stage::admin_proof::mint_daemon_stop_proof_from_env()?);
        return Ok(());
    }
    if !no_verify && !force_unsafe && !assume_merged {
        anyhow::bail!("admin-proof requires at least one privileged completion flag");
    }
    let stage_id = stage_id.expect("clap requires stage_id without --daemon-stop");
    println!(
        "{}",
        stage::complete::mint_completion_proof_from_env(
            &stage_id,
            no_verify,
            force_unsafe,
            assume_merged,
        )?
    );
    Ok(())
}

/// `loom knowledge <subcommand>` dispatch.
///
/// Broken out of `dispatch` because this stage's new context/sync
/// subcommands pushed the parent match past the line limit.
fn dispatch_knowledge(command: KnowledgeCommands) -> Result<()> {
    match command {
        KnowledgeCommands::Update { file, content } => knowledge::update(file, content),
        KnowledgeCommands::ReplaceSection {
            file,
            heading,
            content,
        } => knowledge::replace_section(file, heading, content),
        KnowledgeCommands::Annotate(args) => knowledge::annotate::annotate(args),
        KnowledgeCommands::Context {
            stage,
            query,
            budget_tokens: budget,
            scope,
            require_id,
            history,
            require_compact,
            explain,
            json,
        } => {
            // `budget` is bound short so the context call remains compact.
            knowledge::context::context(
                stage,
                query,
                budget,
                scope,
                require_id,
                history,
                require_compact,
                explain,
                json,
            )
        }
        KnowledgeCommands::Eval {
            cases,
            budget_tokens,
            json,
        } => knowledge::eval::eval(cases, budget_tokens, json),
        KnowledgeCommands::Telemetry { stage, json } => {
            knowledge::telemetry::telemetry(stage, json)
        }
        KnowledgeCommands::Sync {
            structural_only,
            json,
        } => knowledge::sync::sync(structural_only, json),
        KnowledgeCommands::Check { strict, json } => knowledge::check::check(strict, json),
    }
}

/// `loom plan <subcommand>` dispatch.
///
/// Broken out for the same reason as `dispatch_knowledge`: the top-level match
/// sits at its line ceiling, so every new top-level arm has to buy its line back
/// from an existing one.
fn dispatch_plan(command: PlanCommands) -> Result<()> {
    match command {
        PlanCommands::Verify {
            path,
            strict,
            json,
            no_color,
        } => plan::verify::execute(&path, strict, json, no_color),
    }
}

/// `loom context <subcommand>` dispatch.
///
/// These are hook-facing entry points: they run on every agent edit, so they
/// stay quiet on the happy path and degrade instead of failing the tool call
/// that invoked them.
fn dispatch_context(command: ContextCommands) -> Result<()> {
    match command {
        ContextCommands::RecordEdit { stage, paths } => {
            context::record_edit::record_edit(&stage, &paths)
        }
    }
}

/// `loom hook <subcommand>` dispatch.
///
/// The deterministic side of loom's shell hooks: pure filesystem and string
/// work, never a model call and never a network call.
fn dispatch_hook(command: HookCommands) -> Result<()> {
    match command {
        HookCommands::UserPrompt => hook::user_prompt::user_prompt(),
        HookCommands::ReconcileGraph => hook::reconcile_graph::reconcile_graph(),
        HookCommands::PreCompact => hook::pre_compact::pre_compact(),
        HookCommands::ContextCeilings => hook::context_ceilings::context_ceilings(),
        HookCommands::ProjectTypes => hook::project_types::execute(),
    }
}

/// `loom check` dispatch, extracted so `--no-cache` can toggle the acceptance
/// pass cache before running verification without growing the already
/// oversized `dispatch` match arm.
fn run_check(stage_id: String, suggest: bool, no_cache: bool) -> Result<()> {
    if no_cache {
        // Single-threaded at this point in a one-shot CLI invocation — no
        // concurrent reader of the environment exists yet — so a plain
        // process-wide set is safe, the same reasoning
        // `daemon::server::environment::apply` relies on.
        // `CriteriaConfig::default()` (built inside
        // `run_acceptance_with_display`) reads this once.
        std::env::set_var("LOOM_ACCEPTANCE_CACHE", "0");
    }
    verify::execute(&stage_id, suggest)
}

/// `loom {sessions,worktree} <subcommand>` dispatch.
///
/// Extracted for the same reason as `dispatch_knowledge`: grouping these two
/// small, self-contained command groups keeps the top-level `dispatch` match
/// under its line ceiling as new commands are added elsewhere.
fn dispatch_tools(command: Commands) -> Result<()> {
    match command {
        Commands::Sessions { command } => match command {
            SessionsCommands::List => sessions::list(),
            SessionsCommands::Kill { session_ids, stage } => sessions::kill(session_ids, stage),
        },
        Commands::Worktree { command } => match command {
            WorktreeCommands::List => worktree_cmd::list(),
            WorktreeCommands::Remove {
                stage_id,
                force,
                confirmation,
            } => worktree_cmd::remove(stage_id, force, confirmation),
        },
        _ => unreachable!("dispatch routes only the sessions/worktree group here"),
    }
}

/// `loom memory <subcommand>` dispatch.
///
/// Broken out for the same reason as `dispatch_knowledge`: the top-level match
/// sits at its line ceiling, so every new subcommand has to buy its line back
/// from an existing one.
fn dispatch_memory(command: MemoryCommands) -> Result<()> {
    match command {
        MemoryCommands::Note {
            text,
            evidence,
            stage,
        } => memory::note(text, evidence, stage),
        MemoryCommands::Decision {
            text,
            context,
            evidence,
            stage,
        } => memory::decision(text, context, evidence, stage),
        MemoryCommands::Change {
            text,
            evidence,
            stage,
        } => memory::change(text, evidence, stage),
        MemoryCommands::Question {
            text,
            evidence,
            stage,
        } => memory::question(text, evidence, stage),
        MemoryCommands::Resolve {
            event_id,
            outcome,
            target,
            reason,
            stage,
        } => memory::resolve(event_id, outcome, target, reason, stage),
        MemoryCommands::Pending {
            stage,
            json,
            strict,
        } => memory::pending(stage, json, strict),
        MemoryCommands::Query { search, stage } => memory::query(search, stage),
        MemoryCommands::List {
            stage,
            entry_type,
            json,
        } => memory::list(stage, entry_type, json),
        MemoryCommands::Show { stage, all, json } => memory::show(stage, all, json),
    }
}

pub fn dispatch(command: Commands) -> Result<()> {
    match command {
        Commands::Init {
            plan_path,
            clean,
            backend,
            allow_unsafe_plan,
            no_repair,
        } => init::execute(
            Some(PathBuf::from(plan_path)),
            clean,
            backend,
            allow_unsafe_plan,
            no_repair,
        ),
        Commands::Run {
            manual,
            max_parallel,
            foreground,
            watch,
            no_merge,
            backend,
        } => {
            let auto_merge = !no_merge;
            if foreground {
                run::execute(manual, max_parallel, watch, auto_merge, backend)
            } else {
                run::execute_background(manual, max_parallel, auto_merge, backend)
            }
        }
        Commands::Status {
            live,
            compact,
            verbose,
            web_args,
        } => match web_args.web {
            Some(port) => status::web::execute(port, web_args.terminals),
            None => status::execute(live, compact, verbose),
        },
        Commands::Resume { stage_id } => resume::execute(stage_id),
        cmd @ (Commands::Sessions { .. } | Commands::Worktree { .. }) => dispatch_tools(cmd),
        Commands::Attach { stage_id } => attach::execute(stage_id),
        Commands::Graph => graph::show(),
        Commands::Handoff {
            stage,
            session,
            trigger,
            message,
        } => handoff::create::execute(stage, session, trigger, message),
        Commands::Stage { command } => dispatch_stage::dispatch_stage(command),
        Commands::Knowledge { command } => dispatch_knowledge(command),
        Commands::Memory { command } => dispatch_memory(command),
        Commands::Review { ai_summary } => review::execute(ai_summary),
        Commands::Update => self_update::execute(),
        Commands::InstallAssets {
            claude_dir,
            codex_dir,
            skills,
        } => install_assets::execute(claude_dir, codex_dir, skills),
        Commands::Config(args) => config::execute(args),
        Commands::Clean {
            all,
            worktrees,
            sessions,
            state,
        } => clean::execute(all, worktrees, sessions, state),
        Commands::Repair { fix } => repair::execute(fix),
        Commands::Map { args } => map::execute(args),
        Commands::Subagents { args } => subagents::execute(args),
        Commands::Usage { args } => usage::execute(args),
        Commands::Pressure(args) => pressure::execute(args),
        Commands::Stop => stop::execute(),
        Commands::Diagnose { stage_id } => diagnose::execute(&stage_id),
        Commands::Plan { command } => dispatch_plan(command),
        Commands::Check {
            stage_id,
            suggest,
            no_cache,
        } => run_check(stage_id, suggest, no_cache),
        Commands::SkillIndex => skill_index::execute(),
        Commands::Completions {
            shell,
            install,
            migrate,
        } => {
            if migrate {
                return crate::completions::install::check_migration();
            }

            if install {
                let shell = match shell {
                    Some(s) => Shell::from_str(&s)?,
                    None => crate::completions::install::detect_shell()?,
                };
                return crate::completions::install::install(shell);
            }

            let shell = shell.ok_or_else(|| {
                anyhow::anyhow!("Shell argument required. Usage: loom completions <bash|zsh|fish>")
            })?;
            let shell = Shell::from_str(&shell)?;
            generate_completions(shell);
            Ok(())
        }
        Commands::Context { command } => dispatch_context(command),
        Commands::Hook { command } => dispatch_hook(command),
        Commands::Complete { shell, args } => {
            let ctx = CompletionContext::from_args(&shell, &args);
            complete_dynamic(&ctx)
        }
    }
}

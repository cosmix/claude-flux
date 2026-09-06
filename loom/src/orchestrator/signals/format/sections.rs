use crate::handoff::git_handoff::{format_git_history_markdown, GitHistory};
use crate::models::session::Session;
use crate::models::stage::{Stage, StageType};
use crate::models::worktree::Worktree;

use super::super::retrieval::STAGE_QUERY_INPUTS;
use super::super::types::{DependencyStatus, EmbeddedContext};
use super::brief::format_knowledge_brief;
use super::codex::format_codex_implementers_section;
use super::helpers::append_stage_end_sequence;
use super::helpers::{
    extract_tasks_from_stage, format_dependency_outputs, format_dependency_table,
    format_structured_handoff,
};
use super::sandbox_section::format_sandbox_section;
use super::skills::format_skill_recommendations;

/// SEMI-STABLE section: per-stage content (brief, facts), never per-session
pub(super) fn format_semi_stable_section(
    embedded_context: &EmbeddedContext,
    stage_type: StageType,
    stage_id: &str,
) -> String {
    let mut content = String::new();

    // Per-stage `## Knowledge Brief`, gated on retrieval having selected
    // anything. Labelled with the query's INPUT FIELDS, never `pack.query`
    // itself: that is the whole stage description, re-embedded a second time.
    if let Some(pack) = &embedded_context.context_pack {
        content.push_str(&format_knowledge_brief(pack, stage_id, STAGE_QUERY_INPUTS));
    }

    // Stage-type-aware reminder box. Knowledge/integration-verify/distill stages
    // get the knowledge-updates box here; standard stages get their compact
    // memory reminder folded into "## Stage Memory" below instead (kept in one
    // place rather than two - CLAUDE.md already has this content in context).
    if matches!(
        stage_type,
        StageType::Knowledge | StageType::IntegrationVerify | StageType::KnowledgeDistill
    ) {
        content.push_str("```text\n");
        content
            .push_str("┌────────────────────────────────────────────────────────────────────┐\n");
        content
            .push_str("│  📝 KNOWLEDGE UPDATES REQUIRED                                     │\n");
        content
            .push_str("│                                                                    │\n");
        content
            .push_str("│  As you work, UPDATE doc/loom/knowledge/:                          │\n");
        content
            .push_str("│  - Entry points: Key files you discover                            │\n");
        content
            .push_str("│  - Patterns: Architectural patterns you find                       │\n");
        content
            .push_str("│  - Conventions: Coding conventions you learn                       │\n");
        content
            .push_str("│  - Mistakes: Errors you make and how to avoid them                 │\n");
        content
            .push_str("│                                                                    │\n");
        content
            .push_str("│  Command: loom knowledge update <file> \"content\"                   │\n");
        content
            .push_str("└────────────────────────────────────────────────────────────────────┘\n");
        content.push_str("```\n\n");
    }

    // Knowledge Management section with stage-type-aware content
    match stage_type {
        StageType::Knowledge | StageType::IntegrationVerify | StageType::KnowledgeDistill => {
            content.push_str("## Knowledge Management\n\n");

            if embedded_context.knowledge_tree_empty {
                // CRITICAL warning for a knowledge tree with no content at all
                content.push_str("```\n");
                content.push_str(
                    "┌────────────────────────────────────────────────────────────────────┐\n",
                );
                content.push_str(
                    "│  CRITICAL: KNOWLEDGE BASE IS EMPTY                                 │\n",
                );
                content.push_str(
                    "│                                                                    │\n",
                );
                content.push_str(
                    "│  Before implementing ANYTHING, you MUST explore and document:     │\n",
                );
                content.push_str(
                    "│                                                                    │\n",
                );
                content.push_str(
                    "│  1. Entry Points                                                   │\n",
                );
                content.push_str(
                    "│     - Main files, CLI entry, API endpoints                         │\n",
                );
                content.push_str(
                    "│                                                                    │\n",
                );
                content.push_str(
                    "│  2. Architectural Patterns                                         │\n",
                );
                content.push_str(
                    "│     - Error handling, state management, data flow                  │\n",
                );
                content.push_str(
                    "│                                                                    │\n",
                );
                content.push_str(
                    "│  3. Coding Conventions                                             │\n",
                );
                content.push_str(
                    "│     - Naming, file structure, testing patterns                     │\n",
                );
                content.push_str(
                    "│                                                                    │\n",
                );
                content.push_str(
                    "│  4. Mistakes and Lessons Learned                                   │\n",
                );
                content.push_str(
                    "│     - Document errors and how to avoid them                        │\n",
                );
                content.push_str(
                    "│                                                                    │\n",
                );
                content.push_str(
                    "│  This prevents wasted context on repeated exploration.             │\n",
                );
                content.push_str(
                    "└────────────────────────────────────────────────────────────────────┘\n",
                );
                content.push_str("```\n\n");

                content.push_str("**Exploration Order (hierarchical):**\n\n");
                content
                    .push_str("1. **Entry Points First** - Find main.rs, index.ts, app.py, etc.\n");
                content.push_str(
                    "2. **Core Modules** - Identify the key abstractions and data flow\n",
                );
                content.push_str(
                    "3. **Patterns** - Document error handling, logging, config approaches\n",
                );
                content.push_str(
                    "4. **Conventions** - Note naming, file organization, test patterns\n",
                );
                content.push_str("5. **Coverage** - Cover ALL areas you touch — no sampling.\n\n");
            } else {
                // Standard instructions for established knowledge base
                content.push_str("**Extend the knowledge base** as you work:\n\n");
                content.push_str("- Check for undocumented modules in your working area\n");
                content.push_str("- Record new insights about system behavior\n");
                content.push_str("- Document edge cases and gotchas for future sessions\n\n");
            }

            // Show knowledge commands table for Knowledge and IntegrationVerify stages
            content.push_str("**Commands:**\n\n");
            content.push_str("| Discovery Type | Command |\n");
            content.push_str("|----------------|--------|\n");
            content.push_str("| Key entry point | `loom knowledge update entry-points \"## Section\\n\\n- path/file.rs - description\"` |\n");
            content.push_str("| Architectural pattern | `loom knowledge update patterns \"## Pattern Name\\n\\n- How it works\"` |\n");
            content.push_str("| Coding convention | `loom knowledge update conventions \"## Convention\\n\\n- Details\"` |\n");
            content.push_str("| Mistake/lesson | `loom knowledge update mistakes \"## What happened\\n\\n- Details\"` |\n");
            content.push_str("| Detail too long for a tier-1 file | `loom knowledge update <category>/<slug> \"## Section\\n\\n- Details\"` |\n\n");
            content.push_str(
                "**For long content, use stdin:** `loom knowledge update <file> - <<'EOF'`\n\n",
            );

            // Review document generation (knowledge-distill only)
            if stage_type == StageType::KnowledgeDistill {
                content.push_str("**Review Document:**\n\n");
                content.push_str("Generate a change summary for human reviewers:\n");
                content.push_str("```bash\nloom review\n```\n\n");

                // Documentation update reminder (knowledge-distill only)
                content.push_str("```text\n");
                content.push_str(
                    "┌────────────────────────────────────────────────────────────────────┐\n",
                );
                content.push_str(
                    "│  📄 DOCUMENTATION UPDATE REQUIRED                                  │\n",
                );
                content.push_str(
                    "│                                                                    │\n",
                );
                content.push_str(
                    "│  Update user-facing docs to reflect this plan's changes:           │\n",
                );
                content.push_str(
                    "│  - README.md: new commands, features, config, workflows            │\n",
                );
                content.push_str(
                    "│  - CONTRIBUTING.md: new dev patterns, build steps                  │\n",
                );
                content.push_str(
                    "│  - Other docs referencing changed functionality                    │\n",
                );
                content.push_str(
                    "│                                                                    │\n",
                );
                content.push_str(
                    "│  Only update relevant sections. Skip if no user-facing changes.    │\n",
                );
                content.push_str(
                    "│                                                                    │\n",
                );
                content.push_str(
                    "│  ⚠️  System knowledge → loom knowledge update                      │\n",
                );
                content.push_str(
                    "│  Do NOT modify the project's CLAUDE.md — it is the user's file.   │\n",
                );
                content.push_str(
                    "└────────────────────────────────────────────────────────────────────┘\n",
                );
                content.push_str("```\n\n");
            }
        }
        StageType::Standard => {
            // Standard implementation stages: compact memory reminder. Kept
            // short deliberately - CLAUDE.md's own memory rules are already in
            // context every session, so this only needs to point at the
            // commands and the two hard prohibitions, not re-teach the rule.
            content.push_str("## Stage Memory\n\n");
            content.push_str(
                "**SESSION MEMORY REQUIRED — RECORD AS YOU GO.** Record immediately when \
                 these happen, not at stage end - each entry is advice to your future self:\n\n",
            );
            content.push_str("| Trigger | Command |\n");
            content.push_str("|---------|---------|\n");
            content.push_str("| Mistake | `loom memory note \"mistake: ...\"` |\n");
            content.push_str("| Decision | `loom memory decision \"...\" --context \"...\"` |\n");
            content.push_str("| Surprise | `loom memory note \"found: ...\"` |\n");
            content.push_str("| Gotcha | `loom memory note \"gotcha: ...\"` |\n");
            content.push_str("| File change | `loom memory change \"...\"` |\n");
            content.push_str("| Question | `loom memory question \"...\"` |\n\n");
            content.push_str(
                "⚠️ NEVER use 'loom knowledge' in implementation stages — curated into \
                 knowledge by knowledge-distill.\n",
            );
            content.push_str(
                "⛔ NEVER use Claude Code's auto-memory system — ONLY 'loom memory' \
                 commands; anything else is invisible to loom.\n\n",
            );
        }
    }

    // Codex implementer doctrine (semi-stable - gated on codex being one of the
    // stage's licensed lanes, whether or not it is the preferred one)
    if embedded_context.implementers.includes_codex() {
        content.push_str(&format_codex_implementers_section(
            &embedded_context.implementers,
            embedded_context.codex_available,
        ));
    }

    // Per-subagent response budget (semi-stable - gated on an explicit plan value).
    // The orchestrator measures the stage against this budget from the outside, so
    // the session has to be told the same number or it is held to a deadline it
    // cannot see. The general "how do I check on a subagent" doctrine (BLOCK-C) is
    // NOT restated anywhere in the signal - it reaches the agent through
    // `~/.claude/CLAUDE.md` Rule 6 in the same session; this block only layers the
    // stage-specific number on top of that.
    if let Some(timeout_secs) = embedded_context.subagent_timeout_secs {
        content.push_str(&super::helpers::format_subagent_timeout_section(
            timeout_secs,
        ));
    }

    // Ultracode license (semi-stable - gated on the stage's ultracode flag)
    if embedded_context.ultracode {
        content.push_str("## Ultracode Mode\n\n");
        content.push_str(
            "This stage is licensed for ultracode workflow orchestration. Use the Workflow tool\n",
        );
        content.push_str(
            "for the large fan-out this stage was designed for; scale agent count to the work.\n",
        );
        content.push_str(
            "Workflow agents are subagents (Rule 5 applies — no commits, no stage completion);\n",
        );
        content.push_str(
            "the main agent runs acceptance criteria and commits. Do not exceed the stage's\n",
        );
        content.push_str("scope just because orchestration is available.\n");
        content.push_str(
            "Workflow fan-out spawns CLAUDE subagents only: the codex lane (gpt-5.6-terra /\n",
        );
        content.push_str(
            "gpt-5.6-luna) is NOT addressable from a Workflow script. On a stage licensed for\n",
        );
        content.push_str(
            "both, codex work goes through normal `loom-codex-forwarder` Agent spawns, outside\n",
        );
        content.push_str("the Workflow.\n\n");
    }

    // Embed sandbox restrictions (semi-stable - based on stage config)
    if let Some(sandbox_summary) = &embedded_context.sandbox_summary {
        content.push_str(&format_sandbox_section(sandbox_summary));
    }

    // Embed skill recommendations (semi-stable - based on stage description)
    if !embedded_context.skill_recommendations.is_empty() {
        content.push_str(&format_skill_recommendations(
            &embedded_context.skill_recommendations,
        ));
    }

    content
}

/// DYNAMIC section: Changes per session
/// Contains current task, handoff, dependencies, git history
pub(super) fn format_dynamic_section(
    session: &Session,
    stage: &Stage,
    worktree: &Worktree,
    dependencies_status: &[DependencyStatus],
    handoff_file: Option<&str>,
    git_history: Option<&GitHistory>,
    embedded_context: &EmbeddedContext,
) -> String {
    let mut content = String::new();

    // Target section (session-specific)
    content.push_str("## Target\n\n");
    content.push_str(&format!("- **Session**: {}\n", session.id));
    content.push_str(&format!("- **Stage**: {}\n", stage.id));
    if let Some(plan_id) = &stage.plan_id {
        content.push_str(&format!(
            "- **Plan**: {plan_id} (overview embedded below)\n"
        ));
    }
    content.push_str(&format!("- **Worktree**: {}\n", worktree.path.display()));
    content.push_str(&format!("- **Branch**: {}\n", worktree.branch));

    // Add working_dir and computed execution path
    let working_dir = stage.working_dir.as_deref().unwrap_or(".");
    content.push_str(&format!("- **working_dir**: `{working_dir}`\n"));
    let execution_path = if working_dir == "." {
        worktree.path.display().to_string()
    } else {
        format!("{}/{}", worktree.path.display(), working_dir)
    };
    content.push_str(&format!("- **Execution Path**: `{execution_path}`\n"));
    content.push('\n');

    // Execution path reminder box
    content.push_str("```text\n");
    content.push_str("┌────────────────────────────────────────────────────────────────────┐\n");
    content.push_str("│  📍 WHERE COMMANDS EXECUTE                                         │\n");
    content.push_str("│                                                                    │\n");
    content.push_str(&format!(
        "│  Acceptance criteria run from: {}{}│\n",
        execution_path,
        " ".repeat(39_usize.saturating_sub(execution_path.len()))
    ));
    content.push_str(&format!(
        "│  Formula: WORKTREE + working_dir = {}{}│\n",
        working_dir,
        " ".repeat(29_usize.saturating_sub(working_dir.len()))
    ));
    content.push_str("│                                                                    │\n");
    content.push_str("│  If cargo/npm fails with 'not found', check working_dir setting.   │\n");
    content.push_str("└────────────────────────────────────────────────────────────────────┘\n");
    content.push_str("```\n\n");

    // Add worktree root directory reminder (defense-in-depth)
    content.push_str(&format!(
        "**IMPORTANT:** Before running `loom stage complete`, ensure you are at the worktree root: `cd {}`\n\n",
        worktree.path.display()
    ));

    // Worktree Isolation section with explicit boundaries
    content.push_str("## Worktree Isolation\n\n");

    // Show both relative and absolute paths
    let relative_path = format!(".worktrees/{}/", stage.id);
    content.push_str(&format!("You are working in: `{relative_path}`\n\n"));

    // Try to get absolute path for clarity
    if let Ok(absolute_path) = worktree.path.canonicalize() {
        content.push_str(&format!(
            "**Absolute path:** `{}`\n\n",
            absolute_path.display()
        ));
    }

    content.push_str("**ALLOWED:**\n");
    content.push_str("- Files within this worktree\n");
    content.push_str("- `.loom/work/` directory (via symlink)\n");
    content.push_str("- Reading `CLAUDE.md` (symlinked)\n");
    content.push_str("- Using loom CLI commands\n\n");

    content.push_str("**FORBIDDEN:**\n");
    content.push_str("- Path traversal (`../../`, `../.worktrees/`)\n");
    content.push_str("- Git operations targeting main repo (`git -C`, `--work-tree`)\n");
    content.push_str("- Direct modification of `.loom/work/stages/` or `.loom/work/sessions/`\n");
    content.push_str("- Attempting to merge your own branch (loom handles merges)\n\n");

    content.push_str(
        "If you need something outside your worktree, **STOP** and explain what you need.\n",
    );
    content.push_str("The orchestrator will handle cross-worktree operations.\n\n");

    // Embed plan overview if available
    if let Some(plan_overview) = &embedded_context.plan_overview {
        content.push_str("## Plan Overview\n\n");
        content.push_str("<plan-overview>\n");
        content.push_str(plan_overview);
        content.push_str("\n</plan-overview>\n\n");
    }

    // Cross-stage change summary (integration-verify only)
    if let Some(summary) = &embedded_context.cross_stage_summary {
        content.push_str(summary);
        content.push_str("\n\n");
    }

    // Wiring checklist from stage memories
    if let Some(checklist) = &embedded_context.wiring_checklist {
        content.push_str(checklist);
        content.push_str("\n\n");
    }

    // Assignment section
    content.push_str("## Assignment\n\n");
    content.push_str(&format!("{}: ", stage.name));
    if let Some(desc) = &stage.description {
        content.push_str(desc);
    } else {
        content.push_str("(no description provided)");
    }
    content.push_str("\n\n");

    // Dependencies status (dynamic - status changes)
    if !dependencies_status.is_empty() {
        content.push_str("## Dependencies Status\n\n");
        content.push_str(&format_dependency_table(dependencies_status));
        content.push('\n');

        // Include outputs from completed dependencies
        let deps_with_outputs: Vec<_> = dependencies_status
            .iter()
            .filter(|d| !d.outputs.is_empty())
            .collect();

        if !deps_with_outputs.is_empty() {
            content.push_str("## Dependency Outputs\n\n");
            content.push_str(&format_dependency_outputs(&deps_with_outputs));
            content.push('\n');
        }
    }

    // Embed handoff content if available (previous session context)
    if let Some(parsed) = &embedded_context.parsed_handoff {
        // V2 structured handoff: show structured summary
        content.push_str("## Previous Session Handoff (Structured)\n\n");
        content.push_str(&format_structured_handoff(parsed));
        content.push('\n');
    } else if let Some(handoff_content) = &embedded_context.handoff_content {
        // V1 prose handoff: embed raw content
        content.push_str("## Previous Session Handoff\n\n");
        content.push_str(
            "**READ THIS CAREFULLY** - This contains context from the previous session:\n\n",
        );
        content.push_str("<handoff>\n");
        content.push_str(handoff_content);
        content.push_str("\n</handoff>\n\n");
    } else if let Some(handoff) = handoff_file {
        // Fallback reference if content couldn't be read
        content.push_str("## Context Restoration\n\n");
        content.push_str(&format!(
            "- `.loom/work/handoffs/{handoff}.md` - **READ THIS FIRST** - Previous session handoff\n\n"
        ));
    }

    // Git History from previous session (if resuming)
    if let Some(history) = git_history {
        content.push_str(&format_git_history_markdown(history));
        content.push('\n');
    }

    // Acceptance Criteria (stage-specific but part of dynamic for ordering)
    content.push_str("## Acceptance Criteria\n\n");

    // Reminder about working_dir for acceptance criteria
    let working_dir = stage.working_dir.as_deref().unwrap_or(".");
    content.push_str(&format!(
        "**Note:** These commands will run from working_dir: `{working_dir}`\n\n"
    ));

    if stage.acceptance.is_empty() {
        content.push_str("- [ ] Implementation complete\n");
        content.push_str("- [ ] Code reviewed and tested\n");
    } else {
        for criterion in &stage.acceptance {
            content.push_str(&format!("- [ ] {}\n", criterion.command()));
        }
    }
    content.push('\n');

    // Goal-backward verification criteria (if defined)
    if stage.has_any_goal_checks() {
        content.push_str("\n## Goal-Backward Verification\n\n");
        content.push_str("Beyond acceptance criteria, verify these OUTCOMES work:\n\n");

        if !stage.artifacts.is_empty() {
            content.push_str("### Artifacts (files must exist with real implementation)\n\n");
            for artifact in &stage.artifacts {
                content.push_str(&format!("- `{artifact}`\n"));
            }
            content.push('\n');
        }

        if !stage.wiring.is_empty() {
            content.push_str("### Wiring (critical connections to verify)\n\n");
            for check in &stage.wiring {
                content.push_str(&format!(
                    "- **{}**: pattern `{}` in `{}`\n",
                    check.description, check.pattern, check.source
                ));
            }
            content.push('\n');
        }

        // Wiring tests (wiring_tests field)
        if !stage.wiring_tests.is_empty() {
            content.push_str("### Wiring Tests (integration commands)\n\n");
            for test in &stage.wiring_tests {
                content.push_str(&format!("**{}:** `{}`\n", test.name, test.command));
                if let Some(desc) = &test.description {
                    content.push_str(&format!("  *{}*\n", desc));
                }
                let mut criteria = Vec::new();
                if let Some(code) = test.success_criteria.exit_code {
                    criteria.push(format!("exit code: {}", code));
                }
                if !test.success_criteria.stdout_contains.is_empty() {
                    criteria.push(format!(
                        "stdout contains: {}",
                        test.success_criteria.stdout_contains.join(", ")
                    ));
                }
                if !test.success_criteria.stdout_not_contains.is_empty() {
                    criteria.push(format!(
                        "stdout must NOT contain: {}",
                        test.success_criteria.stdout_not_contains.join(", ")
                    ));
                }
                if let Some(true) = test.success_criteria.stderr_empty {
                    criteria.push("stderr must be empty".to_string());
                }
                if !criteria.is_empty() {
                    content.push_str(&format!("  Success: {}\n", criteria.join("; ")));
                }
                content.push('\n');
            }
        }

        // Dead code check (dead_code_check field)
        if let Some(dead_code) = &stage.dead_code_check {
            content.push_str("### Dead Code Check\n\n");
            content.push_str(&format!("**Build command:** `{}`\n", dead_code.command));
            if !dead_code.fail_patterns.is_empty() {
                content.push_str(&format!(
                    "  Fail patterns: {}\n",
                    dead_code.fail_patterns.join(", ")
                ));
            }
            if !dead_code.ignore_patterns.is_empty() {
                content.push_str(&format!(
                    "  Ignore patterns: {}\n",
                    dead_code.ignore_patterns.join(", ")
                ));
            }
            content.push('\n');
        }

        content.push_str("Run `loom check <stage-id> --suggest` to check these automatically.\n\n");
    }

    // Files to modify
    if !stage.files.is_empty() {
        content.push_str("## Files to Modify\n\n");
        for file in &stage.files {
            content.push_str(&format!("- {file}\n"));
        }
        content.push('\n');
    }

    content
}

/// RECITATION section: At end for maximum attention (Manus pattern)
/// Contains immediate tasks, task progression, and session memory
pub(super) fn format_recitation_section(
    stage: &Stage,
    embedded_context: &EmbeddedContext,
) -> String {
    let mut content = String::new();

    // Context usage (high attention position - before tasks). Rendered as an
    // absolute token count against the stage's resident-token ceiling, not a
    // percentage - see `context_ceiling_tokens` on `Stage`/`StageDefinition`.
    if let (Some(tokens), Some(ceiling)) = (
        embedded_context.context_tokens,
        embedded_context.context_ceiling_tokens,
    ) {
        content.push_str(&format!("Context: {tokens} of {ceiling} tokens\n\n"));
    }

    // Immediate tasks - recited at end for attention
    content.push_str("## Immediate Tasks\n\n");
    let tasks = extract_tasks_from_stage(stage);
    if tasks.is_empty() {
        content.push_str("1. Review stage acceptance criteria above\n");
        content.push_str("2. Implement required changes\n");
        content.push_str("3. Verify all acceptance criteria are met\n");
    } else {
        for (i, task) in tasks.iter().enumerate() {
            content.push_str(&format!("{}. {task}\n", i + 1));
        }
    }
    append_stage_end_sequence(&mut content);
    content.push('\n');

    // Embed stage memory at the END for maximum attention (Manus recitation pattern)
    content.push_str("## Stage Memory\n\n");
    if let Some(memory_content) = &embedded_context.memory_content {
        content.push_str("**YOUR WORKING MEMORY** - Notes and decisions from this stage:\n\n");
        content.push_str(memory_content);
        content.push('\n');
    } else {
        // CRITICAL: Show prominent prompt when memory is empty
        content.push_str("```\n");
        content.push_str("┌─────────────────────────────────────────────────────────────┐\n");
        content.push_str("│  ⚠️  NO MEMORY ENTRIES RECORDED — THIS IS A PROBLEM         │\n");
        content.push_str("│                                                             │\n");
        content.push_str("│  You should have been recording memories AS YOU WORKED:     │\n");
        content.push_str("│  - Every mistake/error you hit and how you fixed it         │\n");
        content.push_str("│  - Every non-obvious decision and WHY                       │\n");
        content.push_str("│  - Every surprise or gotcha in the code                     │\n");
        content.push_str("│                                                             │\n");
        content.push_str("│  BEFORE completing this stage, record what you learned:     │\n");
        content.push_str("│  BAD:  \"mistake: wrong path\"  (no context, useless)         │\n");
        content.push_str("│  GOOD: \"mistake: used loom/src/foo.rs in acceptance but     │\n");
        content.push_str("│    working_dir='loom' so path should be src/foo.rs.         │\n");
        content.push_str("│    Prevention: check working_dir before writing paths\"      │\n");
        content.push_str("│                                                             │\n");
        content.push_str("│  Empty memory = lost learning = repeated mistakes           │\n");
        content.push_str("└─────────────────────────────────────────────────────────────┘\n");
        content.push_str("```\n\n");
    }
    content.push_str("**Memory Commands:**\n");
    content.push_str("- `loom memory note \"observation\"` - Record a discovery\n");
    content.push_str(
        "- `loom memory decision \"choice\" --context \"rationale\"` - Record a decision\n",
    );
    content.push_str("- `loom memory question \"open question\"` - Record an open question\n");
    content.push_str("- `loom memory change \"file.rs - description\"` - Record a file change\n");
    content.push_str("- `loom memory list` - Review your stage entries\n");
    content.push_str("- `loom memory show --all` - Show all stage memories\n\n");

    content
}

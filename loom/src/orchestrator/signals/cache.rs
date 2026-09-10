use sha2::{Digest, Sha256};

use super::helpers::append_commit_timing_rules;
use super::helpers::{
    append_completion_rules, append_settled_completion_rules, CONTEXT_CEILING_HANDOFF,
};

mod blocks;
use blocks::{
    append_adversarial_review, append_execution_rules_header, append_isolation_boundaries_simple,
    append_no_verify_block, append_path_boundaries, append_review_dimension_details,
    append_subagent_ceiling_block,
};
mod distill_ordering;
use distill_ordering::append_memory_ordering_doctrine;
// Re-exported so `cache::KNOWLEDGE_CONSUMPTION_CONTRACT` keeps resolving for
// existing callers (e.g. `tests_doctrine_prefixes.rs`) after the move; a
// lib-only build never compiles that `#[cfg(test)]` caller, hence the allow
// (same pattern as `format/mod.rs`'s `extract_tasks_from_description`).
#[allow(unused_imports)]
pub(crate) use blocks::KNOWLEDGE_CONSUMPTION_CONTRACT;

/// Metrics about a generated signal for debugging and optimization
#[derive(Debug, Clone, Default)]
pub struct SignalMetrics {
    /// Total size of the signal in bytes
    pub signal_size_bytes: usize,
    /// Estimated token count (approximate: bytes / 4)
    pub estimated_tokens: usize,
    /// SHA-256 hash of the stable prefix for cache debugging
    pub stable_prefix_hash: String,
    /// Size of stable prefix in bytes
    pub stable_prefix_bytes: usize,
    /// Size of semi-stable section in bytes
    pub semi_stable_bytes: usize,
    /// Size of dynamic section in bytes
    pub dynamic_bytes: usize,
    /// Size of recitation section in bytes
    pub recitation_bytes: usize,
}

impl SignalMetrics {
    /// Compute metrics from signal sections
    pub fn from_sections(stable: &str, semi_stable: &str, dynamic: &str, recitation: &str) -> Self {
        let stable_bytes = stable.len();
        let semi_stable_bytes = semi_stable.len();
        let dynamic_bytes = dynamic.len();
        let recitation_bytes = recitation.len();
        let total_bytes = stable_bytes + semi_stable_bytes + dynamic_bytes + recitation_bytes;

        Self {
            signal_size_bytes: total_bytes,
            estimated_tokens: total_bytes / 4,
            stable_prefix_hash: compute_hash(stable),
            stable_prefix_bytes: stable_bytes,
            semi_stable_bytes,
            dynamic_bytes,
            recitation_bytes,
        }
    }
}

/// Compute SHA-256 hash of content, returning first 16 hex characters
pub fn compute_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();
    hex::encode(&result[..8])
}

// ── Shared content blocks ────────────────────────────────────────────
//
// The `append_*` block helpers and their supporting constants live in
// `cache/blocks.rs` — see that module's doc comment. This section keeps only
// what the generators below need directly.

/// Gate/review pairs interpolated into `append_commit_timing_rules` — one per
/// stage family (code-producing vs. documentation) — so the argument strings
/// are defined once rather than repeated at each of the two call sites.
const CODE_STAGE_GATE: &str = "build, tests, lint, format, plus this stage's acceptance criteria";
const CODE_STAGE_REVIEW: &str = "The mini adversarial code review has RETURNED, every finding is FIXED, and the gate is green AGAIN after those fixes.";
const DOC_STAGE_GATE: &str = "this stage's acceptance criteria";
const DOC_STAGE_REVIEW: &str = "You have re-read every knowledge file you wrote — nothing stale left standing, no duplicate headings — and the acceptance criteria pass AGAIN after any fix.";

// ── Prefix generators ────────────────────────────────────────────────

/// Stable prefix content that rarely changes (Manus KV-cache pattern)
pub fn generate_stable_prefix() -> String {
    let mut content = String::new();

    content.push_str("## Worktree Context\n\n");
    content.push_str("**Isolation Boundaries (STRICT):** this signal is self-contained; you are **CONFINED** here — **STAY IN THIS WORKTREE**, no `git -C`, no `cd ../..`. Your branch merges after `loom stage complete`.\n\n");

    append_path_boundaries(&mut content);

    content.push_str(
        "**working_dir Reminder:** Acceptance criteria execute from `WORKTREE + working_dir` — see the Target section below for the exact path.\n\n",
    );

    append_execution_rules_header(&mut content);
    append_no_verify_block(&mut content);
    append_subagent_ceiling_block(&mut content);
    append_adversarial_review(&mut content);

    content.push_str("**Completion:**\n");
    append_commit_timing_rules(&mut content, CODE_STAGE_GATE, CODE_STAGE_REVIEW);
    append_completion_rules(&mut content);

    content
}

/// Stable prefix for integration-verify stages (final quality gate)
pub fn generate_integration_verify_stable_prefix() -> String {
    let mut content = String::new();

    // Integration-verify header
    content.push_str("## Integration Verification Context\n\n");
    content.push_str(
        "You are running an **integration-verify stage** - the **FINAL QUALITY GATE** before merge.\n\n",
    );

    content.push_str("**ZERO TOLERANCE FOR ISSUES:** **ALL** compiler warnings, linter errors, test failures, and IDE warnings must be FIXED, not suppressed. **NOTHING** is \"pre-existing\", \"out of scope\", or \"too trivial\" — every issue is YOUR issue.\n\n");

    content.push_str("**Your Mission:** **REVIEW** code for quality, security, and correctness; **FIX** every warning and error; **VERIFY** all acceptance criteria pass; **TEST** the feature end-to-end.\n\n");

    // Mini adversarial code review — the six required dimensions, stated up front
    append_adversarial_review(&mut content);
    append_review_dimension_details(&mut content);

    // Isolation + path boundaries (shared)
    append_isolation_boundaries_simple(&mut content);
    append_path_boundaries(&mut content);

    append_execution_rules_header(&mut content);

    append_no_verify_block(&mut content);
    content.push_str(
        "⚠️ **INTEGRATION-VERIFY OVERRIDE — the no-verify rule above does NOT apply here:**\n\n\
         It is written for implementation-stage subagents. IV review/verification subagents are \
         the OPPOSITE case: tell every build/test/sandbox or functional verifier you spawn to run \
         the COMPLETE suite (e.g. `cargo build`, `cargo test`, `cargo clippy -- -D warnings`, \
         `cargo fmt --check`) and read all stderr — that IS their job here.\n\n",
    );
    append_subagent_ceiling_block(&mut content);

    content.push_str("**Completion:**\n");
    append_commit_timing_rules(&mut content, CODE_STAGE_GATE, CODE_STAGE_REVIEW);
    content.push_str(
        "- **Fix ALL issues** - do not mark complete with any warnings or errors remaining\n",
    );
    append_completion_rules(&mut content);

    content.push_str("Knowledge distillation is handled by a separate knowledge-distill stage that runs after this stage.\n\n");

    content
}

/// Stable prefix for knowledge-distill stages (runs in worktree, after integration-verify)
pub fn generate_knowledge_distill_stable_prefix() -> String {
    let mut content = String::new();

    // Knowledge Distillation header
    content.push_str("## Knowledge Distillation Context\n\n");
    content.push_str(
        "You are running a **knowledge-distill stage** that runs AFTER integration-verify, in its own worktree.\n\n",
    );
    content.push_str("Your purpose is to **distill stage memories into permanent knowledge** and **generate the review document**.\n");
    content.push_str(
        "Memories that are not distilled into knowledge are LOST when the plan completes.\n\n",
    );

    // Knowledge distillation workflow
    content.push_str("**Knowledge Distillation Workflow:**\n\n");
    append_memory_ordering_doctrine(&mut content);
    content
        .push_str("1. **RECORD your findings** — As you review code and verify, record your own\n");
    content
        .push_str("   discoveries to `loom memory` (bugs found, security issues, architectural\n");
    content.push_str(
        "   insights, test gaps). These are just as valuable as implementation memories.\n",
    );
    content.push_str("2. Read ALL stage memories (including yours): `loom memory show --all`\n");
    content.push_str(
        "3. Memories are CANDIDATE evidence, not proof — every stage was instructed to record its\n",
    );
    content.push_str(
        "   insights, but a memory is a claim until it is checked. Before writing any code-grounded\n",
    );
    content.push_str(
        "   claim into knowledge, verify it against the FINAL tree: `loom map --find-all <symbol>`\n",
    );
    content.push_str(
        "   or `loom knowledge context --query \"<question>\"` to locate the named lines, then read\n",
    );
    content.push_str(
        "   just those lines — do NOT re-read the whole diff; that is what fills your context.\n",
    );
    content
        .push_str("4. **DISTILL** all memories into `loom knowledge` — synthesize insights from\n");
    content.push_str("   ALL stages (implementation AND your own verification findings):\n");
    content.push_str("   - `architecture` — new components, data flows, integration points\n");
    content.push_str("   - `entry-points` — new files, commands, endpoints added\n");
    content.push_str("   - `patterns` — patterns introduced or discovered during implementation\n");
    content.push_str(
        "   - `conventions` — coding conventions learned from user feedback or code review\n",
    );
    content.push_str("   - `mistakes` — errors made, written as ACTIONABLE PREVENTION RULES: what was misleading, how to detect it, what to do instead. If 2+ stages hit the same mistake, it is a systemic issue — document the root cause\n");
    content.push_str("   - `stack` — new dependencies, tooling changes\n");
    content.push_str("   - `concerns` — tech debt introduced, known issues\n");
    content.push_str("   **Tier routing:** tier-1 files are summaries, not archives. A section under ~40 lines goes inline; a longer one goes to a topic file (`loom knowledge update <category>/<slug> \"...\"`) with a 2-4 line summary plus link left behind. `INDEX.md` is regenerated automatically on every `loom knowledge update` — there is NO index step to run, so finish with your last write.\n");
    content.push_str("5. DO NOT blindly copy memory entries — synthesize and curate\n");
    content.push_str("6. **CORRECTIONS PASS — run it BEFORE the step-4 writes.** Sweep `loom memory show --all` for entries starting `stale-knowledge:` and apply EVERY one, plus anything else you find stale. An unapplied `stale-knowledge:` memory is a correction LOST when this plan completes, and the falsehood is quoted into every later Knowledge Brief. Correct IN PLACE with `loom knowledge replace-section <file> \"<heading>\" \"<corrected body>\"` (body WITHOUT its `## ` heading line) — never `loom knowledge update`, which APPENDS. When no heading matches, `replace-section` appends and SAYS so: read that line, or the stale text is still standing.\n");
    content.push_str("7. **RECEIPT PROTOCOL — every memory event gets exactly one outcome:**\n");
    content.push_str("   a. `loom memory show --all --json` to get every entry's id.\n");
    content.push_str("   b. For EVERY Note/Decision/Question entry, record its outcome: `loom memory resolve <id> --outcome promoted --target <file#heading>` right after the `loom knowledge update`/`replace-section` call that used it; `merged` when it folded into an existing section instead of a new one; `discarded --reason \"...\"` when it is a duplicate, regenerable, or wrong; `deferred --reason \"...\"` when it needs evidence not available now.\n");
    content.push_str("   c. Finish with `loom memory pending --strict` and resolve whatever it lists — nothing may leave this stage unresolved.\n");
    content.push_str("   d. This protocol runs AFTER step 6: the corrections pass stays first.\n");
    content.push_str("8. Generate review document: `loom review`\n\n");

    // Distillation is single-agent work: the curator holds the whole picture.
    content.push_str("**Work single-agent — do NOT spawn subagents:**\n\n");
    content.push_str("Distillation is a linear read-synthesize-write pass and coherence comes from ONE curator holding the whole picture. No gathering agents, no reviewers, no fan-out: you are the only writer, so synthesize, dedupe across categories, and run every `loom knowledge update` yourself. Manage context by leaning on the memories rather than the diff.\n\n");

    content.push_str("**Do NOT modify the project's CLAUDE.md** — it is the user's file. ALL system knowledge goes to `loom knowledge update`.\n\n");

    // Isolation + path boundaries (shared)
    append_isolation_boundaries_simple(&mut content);
    append_path_boundaries(&mut content);

    append_execution_rules_header(&mut content);

    content.push_str("**Completion:**\n");
    append_commit_timing_rules(&mut content, DOC_STAGE_GATE, DOC_STAGE_REVIEW);
    append_completion_rules(&mut content);

    content
}

/// Stable prefix for knowledge stages (runs in main repo, no worktree)
pub fn generate_knowledge_stable_prefix() -> String {
    let mut content = String::new();

    // Knowledge header
    content.push_str("## Knowledge Stage Context\n\n");
    content.push_str(
        "You are running a **knowledge-gathering stage** in the **main repository**.\n\n",
    );
    content.push_str("**Key Differences from Regular Stages:**\n\n");
    content
        .push_str("- **NO WORKTREE** - You are in the main repository, not an isolated worktree\n");
    content.push_str("- **COMMITS REQUIRED** - You MUST `git add doc/loom/knowledge/` and `git commit` before completing\n");
    content.push_str("- **NO MERGING** - Your commits go directly to main (no branch to merge)\n");
    content.push_str(
        "- **EXPLORATION FOCUS** - Your goal is to understand and document the codebase\n\n",
    );

    // Mission
    content.push_str("**Your Mission:**\n\n");
    content.push_str(
        "Build a **briefing document** for future implementation agents. Every entry you\n",
    );
    content.push_str(
        "write should help an agent who has never seen this codebase avoid mistakes and\n",
    );
    content.push_str("find their way quickly. Implementation stages build on this foundation.\n\n");
    content.push_str("1. **Exhaustively map** the codebase (hierarchically) — entry points, every module, data flow, patterns, conventions; leave no major area unmapped.\n");
    content.push_str(
        "2. **Document** findings using `loom knowledge update <file> <content>` commands\n",
    );
    content.push_str("3. **Backfill** any knowledge gaps — if existing knowledge files are sparse, enrich them\n");
    content.push_str("4. **Contextualize the plan** — understand what the plan intends to change and document the current state of those areas\n");
    content.push_str("5. **Review existing mistakes** — pull them with `loom knowledge context --stage <stage-id> --query \"mistakes\" --budget-tokens <n>` and check if any entries are now obsolete or fixed. Remove stale entries to keep the briefing accurate\n");
    content.push_str("6. **Verify** acceptance criteria before completing\n\n");
    content.push_str("**Do NOT modify the project's CLAUDE.md** — it is the user's file. All knowledge goes to `loom knowledge update`; your own insights go to `loom memory`.\n\n");

    append_execution_rules_header(&mut content);

    content.push_str("**Completion:**\n");
    append_commit_timing_rules(&mut content, DOC_STAGE_GATE, DOC_STAGE_REVIEW);
    append_settled_completion_rules(&mut content);
    content.push_str("- **Commit knowledge changes**: `git add doc/loom/knowledge/ && git commit -m 'docs(knowledge): populate codebase knowledge'`\n");
    content.push_str(CONTEXT_CEILING_HANDOFF);
    content.push_str("- **Run `loom stage complete <stage-id>`** when done (from the repo root)\n");
    content.push_str("- **If acceptance criteria fail**: Fix the issues and run `loom stage complete <stage-id>` again\n\n");

    // Knowledge-specific commands
    content.push_str("**Knowledge Commands:**\n\n");
    content.push_str("```bash\n");
    content.push_str("# Update a knowledge file\n");
    content.push_str(
        "loom knowledge update entry-points \"## Section\\n\\n- path/file.rs - description\"\n",
    );
    content.push_str("loom knowledge update patterns \"## Pattern Name\\n\\n- How it works\"\n");
    content.push_str("loom knowledge update conventions \"## Convention\\n\\n- Details\"\n");
    content.push_str("loom knowledge update mistakes \"## What happened\\n\\n- Details\"\n");
    content.push_str("\n# For long content, use heredoc/stdin:\n");
    content.push_str("loom knowledge update patterns - <<'EOF'\n");
    content.push_str("## Section Title\n");
    content.push_str("Content here, can be as long as needed.\n");
    content.push_str("EOF\n");
    content.push_str("\n# Verify what you just wrote — Read the file itself, there is no CLI for this:\n#   tier 1: doc/loom/knowledge/<file>.md\n#   tier 2: doc/loom/knowledge/<category>/<slug>.md\n\n# Pull a scoped brief the way implementation stages will consume it\nloom knowledge context --stage <stage-id> --query \"<question>\" --budget-tokens <n>\n");
    content.push_str("```\n\n");

    content
}

/// Select the stable prefix for a stage type.
///
/// Single source of truth shared by the regular signal path (`format/mod.rs`)
/// and the recovery signal path (`recovery_format.rs`), so a stage resumed via
/// `loom stage recover` / `loom stage retry` gets exactly the same execution
/// rules — including the mini adversarial code review — as a fresh spawn.
pub(crate) fn stable_prefix_for(stage_type: crate::models::stage::StageType) -> String {
    use crate::models::stage::StageType;
    match stage_type {
        StageType::IntegrationVerify => generate_integration_verify_stable_prefix(),
        StageType::KnowledgeDistill => generate_knowledge_distill_stable_prefix(),
        StageType::Knowledge => generate_knowledge_stable_prefix(),
        StageType::Standard => generate_stable_prefix(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_hash_deterministic() {
        let content = "test content";
        let hash1 = compute_hash(content);
        let hash2 = compute_hash(content);
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 16);
    }

    #[test]
    fn test_compute_hash_different_content() {
        let hash1 = compute_hash("content A");
        let hash2 = compute_hash("content B");
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_signal_metrics_from_sections() {
        let stable = "stable content here";
        let semi_stable = "semi-stable";
        let dynamic = "dynamic content";
        let recitation = "recitation at end";

        let metrics = SignalMetrics::from_sections(stable, semi_stable, dynamic, recitation);

        assert_eq!(metrics.stable_prefix_bytes, stable.len());
        assert_eq!(metrics.semi_stable_bytes, semi_stable.len());
        assert_eq!(metrics.dynamic_bytes, dynamic.len());
        assert_eq!(metrics.recitation_bytes, recitation.len());
        assert_eq!(
            metrics.signal_size_bytes,
            stable.len() + semi_stable.len() + dynamic.len() + recitation.len()
        );
        assert_eq!(metrics.estimated_tokens, metrics.signal_size_bytes / 4);
        assert!(!metrics.stable_prefix_hash.is_empty());
    }

    #[test]
    fn test_generate_stable_prefix_contains_required_sections() {
        let prefix = generate_stable_prefix();

        assert!(prefix.contains("## Worktree Context"));
        assert!(prefix.contains("Isolation Boundaries"));
        assert!(prefix.contains("Path Boundaries"));
        assert!(prefix.contains("## Execution Rules"));
        assert!(prefix.contains("STAY IN THIS WORKTREE"));
        // working_dir reminder: acceptance runs from WORKTREE + working_dir
        assert!(prefix.contains("**working_dir Reminder:**"));
        // Completion sequence
        assert!(prefix.contains("When to Commit (ORCHESTRATOR ONLY"));
        assert!(prefix.contains("worktree ROOT directory"));
        // Mini adversarial code review before completion (all six dimensions)
        assert!(prefix.contains("Mini Adversarial Code Review"));
        assert!(prefix.contains("loom-code-reviewer"));
        assert!(prefix.contains("**Code quality & architecture**"));
        assert!(prefix.contains("**Idiomatic code**"));
        assert!(prefix.contains("**Security**"));
        assert!(prefix.contains("**Wiring**"));
        assert!(prefix.contains("**Dead & unnecessary code**"));
        assert!(prefix.contains("**No duplication (DRY)**"));
        assert!(prefix.contains("search the WHOLE codebase"));
        assert!(prefix.contains("tests actually exercise the change"));
        // Per-stage Knowledge Brief consumption contract
        assert!(prefix.contains("Knowledge Brief"));
        assert!(prefix.contains("loom knowledge context --stage"));
        // Subagent no-verify rule (implementation stages do not verify their own work)
        assert!(prefix.contains("VERIFICATION IS THE MAIN AGENT'S JOB - NOT YOURS"));
        assert!(prefix.contains("AT MOST ONE narrowly-scoped check"));
        // Subagent context-ceiling doctrine (BLOCK-D): hook-reported only, never inferred
        assert!(prefix.contains("CONTEXT CEILING - HOOK-REPORTED ONLY"));
        assert!(prefix.contains("SUBAGENT CEILING REACHED"));
        // Regression guard: the IV-only carve-out must NOT leak into the standard
        // prefix. If this ever fails, the override was hoisted into the shared
        // no-verify block and every implementation subagent is now wrongly told
        // to run full build/test/lint suites.
        assert!(!prefix.contains("INTEGRATION-VERIFY OVERRIDE"));
        // Doctrine that reaches the session through ~/.claude/CLAUDE.md must not
        // be restated here - the prefix points at it instead.
        assert!(prefix.contains("Binding rules: ~/.claude/CLAUDE.md"));
        assert!(!prefix.contains("Agent Teams"));
        assert!(!prefix.contains("Subagent Hierarchies"));
        assert!(!prefix.contains("loom subagents watch"));
        assert!(!prefix.contains("git add -A"));
    }

    #[test]
    fn test_stable_prefix_is_stable() {
        let prefix1 = generate_stable_prefix();
        let prefix2 = generate_stable_prefix();
        assert_eq!(prefix1, prefix2, "Stable prefix should be deterministic");
    }

    #[test]
    fn test_knowledge_stable_prefix_contains_required_sections() {
        let prefix = generate_knowledge_stable_prefix();

        assert!(prefix.contains("## Knowledge Stage Context"));
        assert!(prefix.contains("main repository"));
        assert!(prefix.contains("NO WORKTREE"));
        assert!(prefix.contains("COMMITS REQUIRED"));
        assert!(prefix.contains("git add"));
        assert!(prefix.contains("git commit"));
        assert!(prefix.contains("NO MERGING"));
        assert!(prefix.contains("## Execution Rules"));
        assert!(prefix.contains("loom knowledge update"));
        assert!(prefix.contains("loom stage complete"));
        // Points at CLAUDE.md rather than restating it
        assert!(prefix.contains("Binding rules: ~/.claude/CLAUDE.md"));
        assert!(!prefix.contains("Agent Teams"));
        assert!(!prefix.contains("loom subagents watch"));
        // Exhaustive mapping requirement
        assert!(prefix.contains("Exhaustively map"));
        assert!(prefix.contains("leave no major area unmapped"));
        // Per-stage Knowledge Brief consumption contract
        assert!(prefix.contains("Knowledge Brief"));
        // A knowledge write is verified by READING the file - there is no CLI
        // for it, so the prefix must hand over the paths instead of a command.
        // The retired verb is spelled with `concat!` so this file never carries
        // it contiguously: an acceptance criterion greps all of `loom/src` for
        // the deleted commands, and a guard that trips its own check is worse
        // than no guard (same reasoning as tests_doctrine.rs's RETIRED_PHRASES).
        assert!(prefix.contains("doc/loom/knowledge/<category>/<slug>.md"));
        assert!(!prefix.contains(concat!("loom knowledge ", "show")));
        // Documentation stage: emits only markdown, so NO code-review block
        assert!(!prefix.contains("Mini Adversarial Code Review"));
        // Pins the fact that this prefix never calls append_no_verify_block,
        // so it must not carry the implementation-stage no-verify rule.
        assert!(!prefix.contains("VERIFICATION IS THE MAIN AGENT'S JOB"));
        // Knowledge stages run single-agent (no subagents spawned), so the
        // subagent-only ceiling doctrine (BLOCK-D) must not appear either.
        assert!(!prefix.contains("CONTEXT CEILING - HOOK-REPORTED ONLY"));
    }

    #[test]
    fn test_knowledge_stable_prefix_is_stable() {
        let prefix1 = generate_knowledge_stable_prefix();
        let prefix2 = generate_knowledge_stable_prefix();
        assert_eq!(
            prefix1, prefix2,
            "Knowledge stable prefix should be deterministic"
        );
    }

    #[test]
    fn test_integration_verify_stable_prefix_contains_required_sections() {
        let prefix = generate_integration_verify_stable_prefix();

        // Integration-verify specific context
        assert!(prefix.contains("## Integration Verification Context"));
        assert!(prefix.contains("FINAL QUALITY GATE"));
        // Zero tolerance emphasis - the key differentiator
        assert!(prefix.contains("ZERO TOLERANCE"));
        assert!(prefix.contains("ALL"));
        assert!(prefix.contains("NOTHING"));
        assert!(prefix.contains("pre-existing"));
        assert!(prefix.contains("too trivial"));
        // Code review content (merged from code-review prefix)
        assert!(prefix.contains("REVIEW"));
        assert!(prefix.contains("loom-security-audit"));
        assert!(prefix.contains("spawn these as PARALLEL subagents"));
        // Worktree isolation
        assert!(prefix.contains("Isolation Boundaries"));
        assert!(prefix.contains("Path Boundaries"));
        assert!(prefix.contains("CONFINED"));
        // Execution rules
        assert!(prefix.contains("## Execution Rules"));
        // Knowledge distillation moved to separate stage
        assert!(!prefix.contains("Knowledge Distillation (MANDATORY)"));
        assert!(prefix.contains("knowledge-distill stage"));
        // Worktree root directory reminder
        assert!(prefix.contains("worktree ROOT directory"));
        // Review dimension details
        assert!(prefix.contains("Review Dimension Details"));
        assert!(prefix.contains("OWASP Top 10"));
        assert!(prefix.contains("Build/test/sandbox"));
        // Exit code 0 is not success - kept on the dimension that runs the suite
        assert!(prefix.contains("Exit code 0 does NOT mean success"));
        // Mini adversarial code review block (six dimensions stated explicitly)
        assert!(prefix.contains("Mini Adversarial Code Review"));
        assert!(prefix.contains("**Idiomatic code**"));
        assert!(prefix.contains("**No duplication (DRY)**"));
        assert!(prefix.contains("search the WHOLE codebase"));
        // Per-stage Knowledge Brief consumption contract
        assert!(prefix.contains("Knowledge Brief"));
        // Points at CLAUDE.md rather than restating it
        assert!(prefix.contains("Binding rules: ~/.claude/CLAUDE.md"));
        assert!(!prefix.contains("Agent Teams"));
        assert!(!prefix.contains("loom subagents watch"));
        // IV subagents restore full-suite verification: the no-verify rule is present
        // (emitted by `append_no_verify_block`, the BLOCK-A source) AND explicitly
        // overridden for this stage type by the carve-out tail.
        assert!(prefix.contains("VERIFICATION IS THE MAIN AGENT'S JOB - NOT YOURS"));
        assert!(prefix.contains("INTEGRATION-VERIFY OVERRIDE"));
        assert!(prefix.contains("does NOT apply here"));
        assert!(prefix.contains("cargo clippy -- -D warnings"));
        // Subagent context-ceiling doctrine (BLOCK-D): IV also spawns Task-tool
        // subagents (reviewers, verifiers), so it needs the same ceiling rule.
        assert!(prefix.contains("CONTEXT CEILING - HOOK-REPORTED ONLY"));
        assert!(prefix.contains("SUBAGENT CEILING REACHED"));
    }

    #[test]
    fn test_integration_verify_stable_prefix_is_stable() {
        let prefix1 = generate_integration_verify_stable_prefix();
        let prefix2 = generate_integration_verify_stable_prefix();
        assert_eq!(
            prefix1, prefix2,
            "Integration-verify stable prefix should be deterministic"
        );
    }

    #[test]
    fn test_knowledge_distill_stable_prefix_contains_required_sections() {
        let prefix = generate_knowledge_distill_stable_prefix();

        // Knowledge distillation context
        assert!(prefix.contains("Knowledge Distillation"));
        assert!(prefix.contains("loom memory show --all"));
        assert!(prefix.contains("loom knowledge update") || prefix.contains("loom knowledge"),);
        assert!(prefix.contains("loom review"));

        // Isolation and path boundaries
        assert!(prefix.contains("Isolation Boundaries") || prefix.contains("Path Boundaries"),);

        // Must NOT contain IV-specific content
        assert!(!prefix.contains("ZERO TOLERANCE"));
        assert!(!prefix.contains("CODE REVIEW + VERIFICATION"));
        assert!(!prefix.contains("FINAL QUALITY GATE"));
        // Documentation stage: emits only markdown, so NO code-review block
        assert!(!prefix.contains("Mini Adversarial Code Review"));
        // Per-stage Knowledge Brief consumption contract
        assert!(prefix.contains("Knowledge Brief"));
        // Points at CLAUDE.md rather than restating it
        assert!(prefix.contains("Binding rules: ~/.claude/CLAUDE.md"));
        assert!(!prefix.contains("loom subagents watch"));
        // Tier routing, and the fact that the index needs no closing step: an
        // agent told it owes one but given no command will improvise, so the
        // prefix must state the regeneration is automatic.
        assert!(prefix.contains("regenerated automatically on every `loom knowledge update`"));
        assert!(prefix.contains("there is NO index step to run"));
        assert!(prefix.contains("Tier routing"));
        // Pins the fact that this prefix never calls append_no_verify_block,
        // so it must not carry the implementation-stage no-verify rule.
        assert!(!prefix.contains("VERIFICATION IS THE MAIN AGENT'S JOB"));
        // Distillation is single-agent (no subagents spawned), so the
        // subagent-only ceiling doctrine (BLOCK-D) must not appear either.
        assert!(!prefix.contains("CONTEXT CEILING - HOOK-REPORTED ONLY"));
        // Distill runs single-agent on sonnet: the prefix must forbid subagents
        // and must no longer carry the retired fan-out guidance.
        assert!(prefix.contains("Work single-agent — do NOT spawn subagents"));
        assert!(!prefix.contains("information-gathering subagents"));
        assert!(!prefix.contains("If you fan out"));
    }

    #[test]
    fn test_knowledge_distill_stable_prefix_is_stable() {
        let prefix1 = generate_knowledge_distill_stable_prefix();
        let prefix2 = generate_knowledge_distill_stable_prefix();
        assert_eq!(
            prefix1, prefix2,
            "Knowledge-distill stable prefix should be deterministic"
        );
    }
}

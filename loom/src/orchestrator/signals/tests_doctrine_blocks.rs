//! The pinned doctrine literals `tests_doctrine.rs` checks every guidance
//! surface against, plus the phrase table its retirement-regression test
//! sweeps for. Split out purely to keep `tests_doctrine.rs` itself readable
//! and under its line budget - these are DATA the parent's `#[test]`
//! functions assert against, not test logic of their own. Transitively
//! `#[cfg(test)]`: the parent module is gated in `signals/mod.rs`.

/// BLOCK-A - the no-verify rule, verbatim. Every surface must carry this text
/// byte for byte; `hooks/subagent-verify-guard.sh` prefixes it with a hook-only
/// "BLOCKED" framing line, which is deliberately not part of the block.
pub(super) const BLOCK_A: &str = "VERIFICATION IS THE MAIN AGENT'S JOB - NOT YOURS:
- Do NOT verify your work. No full build, no full test suite, no linter, no
  formatter, no type-checker, and never a repeated or looping check.
- AT MOST ONE narrowly-scoped check over the files YOU wrote (e.g.
  `cargo test <your_module>::`), run ONCE. Skip it if you are unsure.
- Report instead: files changed, assumptions made, anything unresolved.
  The MAIN AGENT compiles, tests, lints, and fixes.";

/// BLOCK-B - the model playbook, verbatim.
///
/// A raw literal: the block quotes the phrases an orchestrator uses to talk
/// itself into implementing ("I have diagnosed it"), so it carries `"` inside.
pub(super) const BLOCK_B: &str = r#"1. THE MAIN AGENT NEVER IMPLEMENTS — WHATEVER MODEL IT RUNS (hard stop 6).
   Every stage's main agent is an orchestrator: it decomposes the work, hands
   each subagent full context, then verifies and commits. That is all. This
   holds identically for an opus session and a fable session; a session running
   an expensive model is MORE obliged to delegate, not less.
2. INVESTIGATION ENDS IN A BRIEF, NOT IN AN EDIT. The moment you finish reading
   the code and know what the fix is, you are at the delegation boundary — that
   understanding is exactly what makes a cheap subagent effective. Write it down
   (file:line, root cause, the change to make, signatures, patterns to match,
   acceptance) and spawn. Do not slide from "I have diagnosed it" into "I will
   just type it"; the diagnosis being yours does not make the typing yours.
3. IMPLEMENTATION IS ALWAYS DELEGATED, to as FEW subagents as the work allows, at
   the CHEAPEST tier that can do the piece. Pick PER SUBAGENT by what that piece
   needs, never once for the whole stage, and default downward: codex
   gpt-5.6-luna for boilerplate, scaffolding, and simple unit tests; SONNET
   (loom-software-engineer) or codex gpt-5.6-terra for common implementation and
   integration tests — this is the default lane and most work belongs here; OPUS
   (loom-senior-software-engineer) for mainstream architecture and algorithm
   implementation; FABLE only for visual/UI design, a bug that survived a
   delegated fix attempt, or extremely challenging algorithmic design. Codex
   tiers (effort xhigh, via loom-codex-forwarder) exist only on stages listing
   codex in implementers AND when the codex CLI + plugin are installed;
   otherwise that work goes to sonnet (loom warns at startup when a stage lists
   codex it cannot use). Verification NEVER delegates - the orchestrator
   verifies and commits. Spawn BY AGENT TYPE.
4. ESCALATE ON EVIDENCE, NOT ON HUNCH. Start at the cheapest plausible tier. A
   fix that failed ONCE against clear acceptance criteria moves up exactly one
   tier — sonnet to opus, opus to fable — with the failed attempt and its
   evidence in the new brief; never rerun the same tier on the same bug. "This
   feels subtle" does not justify escalation. When a cheap subagent's output is
   wrong, first ask whether the brief was detailed enough — a vague brief is an
   orchestrator failure, not evidence the tier was too small.
5. DEBUGGING OR REPEATED FAILURE → spawn a `loom-advisor` (fable) subagent:
   narrow scope, full detail supplied by the orchestrator, advice returned, no
   writes. Its diagnosis then feeds a sonnet or opus implementer per point 2.
   Do not let an implementer thrash on the same failure twice."#;

/// BLOCK-D - the subagent context-ceiling rule, verbatim. Unlike BLOCK-A/B/C,
/// this one is pinned against BOTH a static surface (`CLAUDE.md.template`)
/// and the emitted signal prefixes (`cache.rs::append_subagent_ceiling_block`)
/// in the SAME test, because a subagent's route to it cannot rely on either
/// alone: it may never see the literal prose the orchestrator was told to
/// paste, so the signal-side copy is the fallback that reaches it anyway.
pub(super) const BLOCK_D: &str = "CONTEXT CEILING - HOOK-REPORTED ONLY:
- Your ceiling is roughly 800,000 tokens, the same number your orchestrator gets - reading a handful of files never gets you close.
- The hook line beginning `SUBAGENT CEILING REACHED:` in your own tool output is the sole evidence you reached it. Never estimate, infer, or assume one.
- A turn that ends with zero files written, on a task that asked for files, counts as a FAILED unit of work even if the report reads well. If genuinely blocked, name the real blocker - the hook line above is the only thing that counts as a context blocker.";

/// Phrasing RETIRED doctrines used, across the no-verify rule (BLOCK-A) and the
/// subagent-waiting rule (BLOCK-C). Acceptance criteria only grep for the
/// wording a doctrine INTRODUCES, so they cannot catch a surface that still
/// carries the instruction it replaced - which is exactly how the enforcement
/// layer once landed while three guidance files still told subagents to run
/// the suite. Grep for what was retired, not only for what was added.
///
/// Each phrase is assembled with `concat!` so this file never contains one as a
/// contiguous literal. The plan's own acceptance criteria grep this very
/// directory for the retired wording and must find nothing; a checker that
/// trips over its own checklist is worse than no checker. Keep them split, and
/// keep the split out of the middle of a word so the phrase stays readable.
pub(super) const RETIRED_PHRASES: &[&str] = &[
    concat!("verify your ", "subtree"),
    concat!("verifies its ", "subtree"),
    concat!("Test as ", "you go"),
    concat!("Zero IDE ", "diagnostics"),
    concat!("haiku stays rare", " and trivial"),
    concat!("haiku (rare, trivial ", "mechanical edits)"),
    concat!("take the work ", "over"),
    concat!("report back ", "within"),
    concat!("hard ceiling on any", " single check"),
    // BLOCK-C: superseded by `loom subagents watch`'s own deadline/state.
    concat!("a check firing is NOT", " a deadline"),
    concat!("MUST carry a liveness", " signal"),
    // Retired: CLAUDE.md is already in the subagent's context, so ordering it
    // to re-read the file spends a read on text the agent already has.
    concat!("READ CLAUDE.md ", "IMMEDIATELY"),
    // Retired with the token ceiling: the trigger is the PostToolUse hook's
    // message, not a percentage the session watches for itself.
    concat!("if context ", "exceeds 75%"),
    // Retired with the one-background-watch rule: re-arming a foreground watch
    // is a poll loop with extra steps.
    concat!("Re-arm `watch` and keep", " waiting"),
    // Retired with the ceiling raise (both main and subagent now 800,000,
    // one fraction of the shared 1M window): the old fixed 150k figure must
    // never return to doctrine text - the ceiling is model-derived, reported
    // by the hook, never a hardcoded number a session compares itself
    // against.
    concat!("150,", "000"),
    // Retired along with the differentiated-ceiling framing this doctrine
    // briefly carried: a subagent's ceiling is NOT smaller than its parent's
    // - both are 800,000, told apart only by which hook line reports them.
    // Guards against the split creeping back in.
    concat!("600,", "000"),
];

# Doctrine And Acceptance

> Why a one-phrase grep proves presence but never agreement, and how doctrine drifts across surfaces unnoticed.

## An Acceptance Criterion That Greps One Phrase Proves Presence, Never Agreement (2026-07-28)

**What happened:** the no-verify doctrine ("BLOCK-A") had to appear identically on three
surfaces. At integration-verify time `hooks/subagent-verify-guard.sh` carried a 15-line
_reconstruction_ while `orchestrator/signals/cache.rs` and `CLAUDE.md.template` carried the
authoritative 7-line block. **Every acceptance criterion passed**, because they all
`rg -qF` a single anchor phrase that both wordings happened to contain.

**Why:** presence of a substring says nothing about the rest of the text. N greps across N
surfaces still only prove N substrings exist, never that the surfaces agree.

**Prevention:** when the same block must exist verbatim on multiple surfaces, pin it with a
**cross-surface byte-equality test**, not with greps — `include_str!` each static surface,
call the generator for each runtime surface, and assert equality.

**Fix:** `loom/src/orchestrator/signals/tests_doctrine.rs`.

## A Checker That Enumerates Forbidden Strings Must Not Contain Them Contiguously (2026-07-28)

**What happened:** the new cross-surface test lists the _retired_ phrasing it greps for, and it
lives in `loom/src/orchestrator/signals/` — exactly the directory a plan criterion scanned for
that phrasing. Both the constant array and a doc comment quoting it matched, so the test tripped
the plan's own acceptance.

**Prevention:** build each forbidden literal with `concat!` so it never appears contiguously in
the source, and keep the forbidden text out of comments too. **Detection:** run the plan's own
greps against your new file before assuming it is inert.

## An Exception Must Live in Every Block That Gets COPIED (2026-07-28)

**What happened:** the integration-verify carve-out was written into the signal-level override,
which is addressed to the _main agent_ ("when you spawn a verifier, tell it to run the complete
suite"). But what an integration-verify main agent actually pastes into its verifier subagent's
prompt is the Rule 5 preamble from `CLAUDE.md.template` — which carried the doctrine with **no
exception**. The verifier therefore received "no full build, no full test suite" as its most
rule-shaped instruction.

**Prevention:** for each rule, ask **which surface is pasted verbatim into a subagent prompt**,
and check that the exception survives that paste. A doctrine's exception belongs in every block
that is copied, not only in the prose that explains it.

## After Landing a Doctrine, Grep for the Phrasing It RETIRES (2026-07-28)

**What happened:** four subagents each landed the new doctrine correctly in their own territory,
while the phrasing it _replaces_ — "verifies its subtree", "DO write code, run tests",
"test results" — survived a line or two away in `cache.rs`, `format/sections.rs` and
`CLAUDE.md.template`, text nobody was assigned to touch. The enforcement layer shipped while
the guidance layer still instructed the blocked behaviour.

**Why:** acceptance criteria grep for the wording a change _introduces_. Nothing greps for the
wording it _removes_, so contradictions are invisible to the gate.

**Prevention:** after landing a doctrine, sweep the **entire** guidance surface for the retired
phrasing, not just for the new phrasing. A stage's own guidance files can contradict the
doctrine that stage is landing.

## Canonical Text Referenced by Plan Path Is Unreachable From a Worktree (SYSTEMIC, 2026-07-28)

**What happened:** two independent stages were told to copy canonical wording **verbatim from
`doc/plans/`**. Plan files live in the main repo working tree and are frequently uncommitted, so
they are absent from the worktree checkout, and `worktree-file-guard.sh` hard-blocks the
absolute main-repo path on `PreToolUse:Read`. One stage found a readable copy; the other
reconstructed the text from the single phrase pinned by an acceptance criterion, and the two
copies then drifted (see the first lesson on this page).

**Misleading signal:** the signal header claims "plan overview embedded below", but no overview
section is generated.

**Prevention (for plan authors):** any verbatim text a stage must reproduce — canonical wording,
message blocks, doctrine — **must be inlined into the stage description itself**, never
referenced by plan path. **Detection (for executing agents):** if the assignment says "copy
EXACTLY" and the text is not in your signal, you are already blocked — say so rather than
reconstructing.

## Acceptance Runs in the Stage's Sandbox, So a Gate Verified on the Host Strands a Finished Stage (2026-08-17)

**What happened:** `01-overlay-contract` finished its work, passed its own review, committed —
and then could not complete. Two of its acceptance criteria were unpassable inside a worktree
session and always had been. `cargo test --all-targets --no-fail-fast` pulls in the tmux e2e
suite, which cannot create an `AF_UNIX` socket under a session sandbox. And
`./target/debug/loom map --outline src/main.rs` opens `ContextStore`, which resolves its cache
under the MAIN project root — read-only from a worktree. Neither has anything to do with the
stage's diff.

**Why:** `loom stage complete` runs the acceptance list from the agent's own process, so every
criterion inherits the session sandbox. The daemon's host-side verification does not, which is
why the same list looks green from an operator shell. The plan author had confirmed the
`loom map` criterion by running it — in the main checkout, unsandboxed — and had written into
the same plan, correctly, that no `allow_write` line can grant those paths. The finding was
recorded as a caveat instead of disqualifying the command.

**Prevention:** run every acceptance command from a worktree under the stage's own sandbox
before it enters a plan, and record the observed baseline in the plan prose. Treat "this write
can never be granted" as a disqualification of the command, never a caveat to note beside it.
Environment-dependent tests get a self-skip guard rather than a place in any stage's gate.
Detection: a stage whose acceptance fails on files its diff never touched.

**Fix:** the agent cannot force completion and should not try — `--no-verify` needs a one-time
operator proof from `.work/admin.token`, which the sandbox denies by design. But stopping is not
the whole move: **a stage agent that judges a criterion impossible rather than merely failing
should file `loom stage dispute-criteria <stage-id> --criterion-index <n> --reason "..."`**, which
routes through the daemon to adjudication and can amend the criterion via the audited
`apply_amendment` path. Operator-side, `loom stage amend <stage-id> --field acceptance --op
replace --index <n> --value '<cmd>'` reaches the same machinery directly for a stage that is not
currently executing. Reserve both for impossible criteria — a criterion that is merely red is a
defect to fix, not to amend away.

## Paths: working_dir Mismatch (Recurring)

**Mistake:** Acceptance criteria, artifact paths, and file checks used absolute paths like `loom/src/...` when `working_dir` was already `loom`, producing double-paths like `loom/loom/src/...`. Occurred in 5+ separate plans.
**Fix:** ALL paths in acceptance/artifacts/wiring/wiring_tests are relative to `working_dir`. If `working_dir: "loom"`, use `path/to/file.rs` not `loom/path/to/file.rs`. Set `working_dir` to where `Cargo.toml`/`package.json` lives.

## Stages: Marked Complete Without Implementation (Recurring)

**Mistake:** Multiple stages were marked Completed with no code committed. `stage_type: knowledge` auto-sets `merged=true` which masked missing work.
**Fix:** Always run acceptance criteria BEFORE marking stages complete. Verify actual artifacts exist.

## loom merge Command Removal

**Lesson:** `loom merge` duplicated `loom stage complete` functionality with 5 bugs. Removed entirely rather than fixing. When a command duplicates existing functionality and has multiple bugs, removal is better than repair.

## Truths → Acceptance Unification

**What happened:** truths and truth_checks were separate fields on StageDefinition/Stage that overlapped with acceptance criteria. Unified into AcceptanceCriterion enum (Simple|Extended).

**Gotcha:** Old plans with a top-level `truths:` field are now rejected as unknown instead of silently dropping the checks. Migrate those commands to `acceptance`; `before_stage` and `after_stage` remain valid delta-proof fields.

**How to avoid:** Keep plan structs strict with `deny_unknown_fields` so removed or misspelled policy cannot false-pass. When compatibility is required, use an explicit migration path rather than permissive deserialization.

## Stale Acceptance Criteria Referencing External Plan Files

**What happened:** An `integration-verify` stage had an acceptance criterion `cargo run -- plan verify ../doc/plans/DONE-PLAN-cwd-knowledge-resolution.md`. That plan file was deleted during housekeeping (`doc: remove completed plans`) AFTER the stage was authored but BEFORE it ran. The criterion failed at execution time with a file-not-found error, requiring `--no-verify` to complete.

**Why:** Plan files in `doc/plans/` are subject to archiving/deletion as a normal maintenance operation. A file that exists when you write a criterion may not exist when the stage executes, especially for long-running plans.

**Prevention:** When generating acceptance criteria for `integration-verify` stages, never reference plan files from `doc/plans/` directly. Instead, use self-contained fixtures: create a temp file via `TempDir` + `write_plan` in Rust tests (see `tests/integration/plan_verify.rs` for the pattern). If a live-CLI smoke test is needed, write a minimal inline plan to a temp path rather than relying on a file that may be archived.

**Fix:** Use test fixtures that are fully controlled by the test suite. Reference `tests/integration/plan_verify.rs` as the canonical example of building plan fixtures without touching `doc/plans/`.

## Aggregated Wiring Re-Verification: Double-Applied working_dir

**What happened:** `run_aggregated_wiring_reverification` in `commands/stage/complete.rs` was called with `acceptance_dir` (already resolved to `worktree_root + integration-verify.working_dir`) and then joined each prior stage's `working_dir` on top, producing paths like `loom/loom/src/...`. The wiring check reported "Wiring source file missing" for every prior stage.

**Why:** `acceptance_dir` is computed as `worktree_root + working_dir`, so it is already a fully resolved path. Joining another `working_dir` on top re-applies it.

**Prevention — Detection rule:** Any code path that loops over prior stages and builds a source-file path MUST start from `worktree_root`, then join the per-stage `working_dir`. Never start from an already-resolved `acceptance_dir`.

**Fix:** Changed call site to pass `worktree_root` (from `StageExecutionPaths`) through `run_verification_phase` into the aggregated re-verifier; each stage's `working_dir` is joined against the worktree root.

## Vendored slash-command / Codex skill must consume the plan arg verbatim (no `doc/plans/` prefix)

**What happened:** Originally-installed `pressure.md` / codex `SKILL.md` used `doc/plans/$1`, which double-prefixed into `doc/plans/doc/plans/PLAN-foo.md`.
**Why:** The `loom pressure` driver hands children the FULL repo-relative invocation (e.g. `doc/plans/PLAN-foo.md`) because they run with `current_dir(repo_root)`. The template then re-prefixed `doc/plans/`.
**Prevention:** When a Rust driver passes a repo-relative path to a slash command or Codex skill, the template MUST use `$1`/`<PLAN>` directly. The driver owns path resolution (`resolve_plan_path`); the template owns none.
**Fix:** vendored `commands/{pressure,address}.md` and `codex/skills/pressure/SKILL.md` use the arg verbatim.

## Gate path resolution on `is_file()`, not `exists()`, before spawning agents

**What happened:** Plan resolution risked accepting a directory argument.
**Why:** `Path::exists()` is true for directories; canonicalizing one and handing it to claude/codex fails confusingly downstream.
**Fix:** `resolve_plan_path` gates on `is_file()` so a directory arg fails cleanly at resolution.

## A `--dry-run` that hand-builds its command string drifts from the real spawn

**What happened:** An early dry-run printed simplified commands missing `--permission-mode`/`--model`/`-C`.
**Why:** Preview re-derived argv independently of the spawn path.
**Prevention/Fix:** share ONE argv builder between preview and spawn (`claude_args`/`codex_args` feed both `render_dry_run` and `spawn_*`). Any preview that re-derives argv is a silent-divergence hazard.

## Stage signal did not embed the plan's inline command/skill bodies

**What happened:** The implement-pressure signal omitted the plan's inline slash-command and codex-skill bodies. Canonical sources had to be recovered from `~/.claude/commands/*.md` (Read tool) and `~/.codex/skills/pressure/SKILL.md` (Bash `cat` — `worktree-file-guard.sh` ALLOWS the Read tool on `~/.claude/` but BLOCKS it on `~/.codex/`). The installed copies were also STALE.
**Prevention:** When a stage depends on file bodies that live outside the worktree, do not trust the signal to inline them or the installed copies to be current — recover from the authoritative source and treat installed versions as suspect.

## Plans-location rule was prose-only — the one hard rule with no hook enforcement (2026-07-06)

**What happened:** Opus repeatedly wrote plans to `~/.claude/plans/` despite CLAUDE.md.template stating the ban three times (Rule 1, a HARD STOP banner, and the end-of-file reminders).
**Why:** Plan mode injects its save-location suggestion at the moment of the Write call; a prohibition stated mid-file thousands of tokens earlier reliably loses to an instruction present at the decision point. Every other hard rule (commit/complete, git add -A, worktree isolation) had a hook backstop — plans did not: `worktree-file-guard.sh` exits early outside loom worktrees and explicitly whitelists all `~/.claude/**` paths, so interactive sessions (where plan mode runs) had zero deterministic coverage.
**Prevention:** A rule that must never be violated needs a deterministic channel, not more prose. Prose emphasis is also zero-sum — when ~20 rules carry ⛔/NEVER banners, the salience gradient is flat and the load-bearing rules don't stand out.
**Fix:** Added `hooks/plans-path-guard.sh` (PreToolUse on Write|Edit, blocks `.claude/plans` and `.claude/projects/*/plans` path segments, exit-2 message redirects to `doc/plans/`), wired via `fs/permissions/constants.rs`, `fs/permissions/hooks.rs`, and `install.sh`. Restructured CLAUDE.md.template to a 5-item hard-stop tier stated verbatim at top and bottom.

## Delta-Proof `before_stage` Gate Re-Run on Every Re-Spawn Deadlocks the Stage (2026-07-27)

**What happened:** `start_stage` ran a stage's `before_stage` truth checks on _every_ spawn attempt. Those checks are delta-proofs — they assert the feature does NOT exist yet. After a session was interrupted mid-stage (leaving its implementation in the worktree/branch), orphan recovery re-queued the stage, the checks re-ran, found the feature present, and marked the stage `Blocked` with `FailureType::TestFailure` **before spawning any session**. Since no session ever ran, nothing could finish or commit the work, and `loom stage retry` / the next `loom run` reproduced the identical failure forever. The stage could not self-heal.

**Misleading signal:** the failure output is a genuine, correctly-computed check result — "this command exited 0 but the plan says it should exit 1" — so the block looks like a real pre-condition violation rather than the orchestrator tripping over its own prior progress. The comment on the call site ("verify pre-conditions in fresh worktree") described an assumption — a _fresh_ worktree — that `get_or_create_worktree` stops honoring the moment a stage is retried.

**Why:** a one-shot gate was placed on a path that runs many times. Every re-entry route into `start_stage` (orphan recovery → Queued, `loom stage retry`, crash auto-retry) reuses the same worktree and the same `loom/<stage-id>` branch, so the "before" state the check asserts is by construction no longer true after the first attempt.

**Prevention — detection rule:** any check whose _expected_ outcome changes once the stage does its work (delta-proofs, "feature absent" assertions, baseline captures) must be gated on evidence that no work exists yet — not merely placed before the spawn. Before adding a blocking check to a spawn path, ask what it does on attempt #2. And a blocking transition that happens _before_ a session is spawned deserves extra scrutiny: nothing downstream can clear it, so a wrong block is permanent, not merely slow.

**Fix:** `stage_executor.rs::before_stage_gate_passed` calls `verify::before_after::find_prior_stage_work` first and skips the checks (logging the evidence) when the stage branch has commits beyond its resolved base or the worktree has non-scaffold changes. Loom's own worktree scaffolding (`.work`, `.claude/`, root `CLAUDE.md`) is discounted via `git::worktree::is_worktree_scaffold_path` — otherwise, in a repo that doesn't gitignore those, the very first spawn would look "dirty" and silently disable the gate. Note `git::has_uncommitted_changes` excludes untracked files and was useless here (a brand-new module is untracked); `list_working_tree_changes` was added for the "has anyone worked here?" question.

## Stage Fragmentation: Compile-Order Is Not a Stage Boundary (2026-08-07)

**What happened:** the most common loom plan-authoring error is splitting ONE cohesive feature into
one stage per architectural layer (schema → runtime → doctrine → tests) because each layer imports
the one before it. This plan deliberately did not: the whole codex-implementer feature shipped as a
SINGLE standard stage between the knowledge/integration-verify bookends, with a foundation edit
followed by parallel subagents over disjoint files.

**Why:** "B imports A" is a COMPILE-ORDER dependency, and one stage resolves it for free by writing
A first — that is a foundation step, not a stage boundary. Only a MERGE-ORDER dependency is a real
boundary: the dependent work must run against _merged, gate-passed_ code. Each extra stage costs a
worktree, a session, a merge, and a FULL re-run of the acceptance gate.

**Prevention (detection rule):** if a plan has one stage per architectural layer of a single feature
and their `files:` sets are DISJOINT, it is fragmented — merge them. Disjoint file sets are evidence
_for_ merging (parallel subagents can own them), not against it. `/loom-plan-writer` now enforces
this: the Stage Necessity Test (`skills/loom-plan-writer/SKILL.md:388`) requires every non-bookend
stage to name which of Q1-Q4 forced it, the validation checklist re-checks that at `:825`, and
`:771` states outright that a compile-order dependency is a foundation step, not a stage split. A
stage that cannot cite a question is fragmentation.

**Fix:** one stage, foundation edit first, then fan out. Derive the foundation sweep from
`cargo build`, never from the plan's hand-counted file table — in execution the foundation step had
to fix three `Stage` struct literals, not the one the plan named, before fan-out could compile.

## Premature Stage Completion and Deadline Takeovers (2026-08-14)

**What happened:** Stages ran `loom stage complete` while subagents were still out or defects
unfixed, then kept working past completion — post-completion edits are lost because the merge
starts from the completed commit. Separately, orchestrators treated the 300s bounded-check
cadence as a deadline on subagent work and took over or re-spawned live subagents, duplicating
work already paid for.
**Why:** The Stop hook (commit-guard.sh) fires during subagent waits and read as "commit and
complete NOW"; CLAUDE.md Rule 6 said "on the deadline branch: take the work over"; no surface
said completion requires a settled stage or that completion is terminal.
**Prevention:** `hooks/stage-terminal-guard.sh` blocks Write/Edit/Task/Agent in a worktree whose
stage is already completed/verified. Settled-state completion doctrine lives in
CLAUDE.md.template (hard stop 3, Rule 4, Rule 6), `append_completion_rules()` in
`signals/cache.rs`, the budget-exceeded recitation box, and the commit-guard message; retired
takeover phrases are pinned in `tests_doctrine.rs::RETIRED_PHRASES`.
**Fix:** Complete only a settled stage (subagents absorbed, defects fixed, tree clean) and run
nothing after `loom stage complete`; re-arm bounded checks on live subagents and take over only
on positive evidence of death.

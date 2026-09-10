# Doctrine Cross Surface

> Pinning multi-surface guidance with equality tests, ambiguity-equals-fail-safe privilege lookups, and token-based shell classification.

## Doctrine Block Cross-Surface Pinning

A **doctrine block** is a fixed chunk of agent guidance that must appear byte-identically on
several surfaces at once — a runtime signal prefix, a static template, and a hook's refusal
message. loom carries **four** named, positively-pinned blocks. An earlier version of this
section said there were three and that `BLOCK_D` did not exist; that was already wrong when
written or went stale immediately after — `BLOCK_D` is defined at
`orchestrator/signals/tests_doctrine_blocks.rs:65` and pinned by
`tests_doctrine.rs::block_d_agrees_across_every_surface`. The same correction moved `BLOCK_A`
and `BLOCK_B`: their constants live in `tests_doctrine_blocks.rs`, not in `tests_doctrine.rs`
where this table used to point.

| Block     | Content                                                                          | Const location                                   | Pinning test(s)                                                                                              |
| --------- | --------------------------------------------------------------------------------- | ------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| `BLOCK_A` | "VERIFICATION IS THE MAIN AGENT'S JOB — NOT YOURS" (subagent no-verify rule)      | `orchestrator/signals/tests_doctrine_blocks.rs:11` | `tests_doctrine.rs` asserts `text.contains(BLOCK_A)` on every guidance surface                                  |
| `BLOCK_B` | Model allocation / delegation ladder, incl. `CODEX_IMPLEMENTER_MODEL_TERRA`/`_LUNA`/`_EFFORT` | `orchestrator/signals/tests_doctrine_blocks.rs:23` | same file; also asserts `BLOCK_B.contains(...)` for each codex identifier constant, so a renamed codex tier fails here first |
| `BLOCK_C` | Subagent one-background-watch / bounded-wait doctrine ("Checking on subagents...")| `orchestrator/signals/tests_doctrine_waiting.rs:35` | `tests_doctrine_waiting.rs::block_c_names_only_the_frozen_subagents_cli_surface` and friends                    |
| `BLOCK_D` | Subagent context-ceiling rule ("CONTEXT CEILING - HOOK-REPORTED ONLY")            | `orchestrator/signals/tests_doctrine_blocks.rs:65` | `tests_doctrine.rs::block_d_agrees_across_every_surface`                                                         |

`BLOCK_D` is pinned in `tests_doctrine.rs` rather than split into its own sibling because it is
the only block spanning both kinds of surface: a static one (`CLAUDE.md.template` Rule 5) and the
emitted signal (`cache.rs`'s `append_subagent_ceiling_block`). `BLOCK_A`/`BLOCK_B` are
static-only and pinned in the same file; `BLOCK_C` is static-only and pinned in the sibling. It
exists because a subagent that never sees the PostToolUse hook's literal `SUBAGENT CEILING
REACHED` line has no falsifiable way to know it has NOT reached its ceiling, and was observed
confabulating one from CLAUDE.md prose alone.

`CLAUDE.md.template` is `include_str!`'d into both `tests_doctrine.rs` and `tests_doctrine_waiting.rs` (and `tests_doctrine_prefixes.rs`, which separately pins the four stable-prefix generators' byte ceilings — see [signal-generation.md](../architecture/signal-generation.md)) so a change to the template is checked against all four blocks and every generated signal in one pass.

The failure mode is drift, and it is invisible to ordinary acceptance criteria: greping each
surface for an anchor phrase proves only that a substring exists on each, never that the
surfaces agree. Two independently-worded copies both pass.

**The pattern:**

1. **One authority, and the orchestrator writes the foundation text first.** Exactly one surface
   is canonical — CLAUDE.md.template for a rule, or the stage's own brief for a stage-scoped
   change. When a new mandated rule directly contradicts an existing pinned block (real example:
   a stage brief mandated ONE background `loom subagents watch --timeout 3600`, which contradicted
   `BLOCK_C`'s old `(deadline <=300s ...)` parenthetical and its "re-arm and keep waiting" case),
   the fix is to REWORD the pinned block to agree with the new rule — never to leave the block
   byte-frozen and let it contradict live doctrine, and never to have a subagent freelance new
   foundation text. Reconcile *toward* the authoritative source, and update every pinning test's
   asserted needles to match the reworded block, keeping only the substrings that must survive.
2. **Pin with equality, not greps.** A single test `include_str!`s each static surface, calls
   the generator for each runtime surface, and asserts byte equality (or, for `BLOCK_B`,
   substring containment of the identifiers it must carry) against the canonical block. See
   `loom/src/orchestrator/signals/tests_doctrine.rs` and `tests_doctrine_waiting.rs`.
3. **Exceptions travel with the copied block.** For every rule ask: *which surface is pasted
   verbatim into a subagent prompt?* Any carve-out must survive that paste. A carve-out that
   lives only in the prose explaining the rule never reaches the agent that needs it.
4. **Frame outside the block.** Surface-specific framing (a hook's `BLOCKED:` header line,
   language-specific examples) sits *outside* the pinned block so the block itself stays
   byte-identical everywhere.
5. **Sweep for retired phrasing with a NEGATIVE pin, not a memory of the grep.** `RETIRED_PHRASES`
   in `tests_doctrine.rs` is asserted absent from `guidance_surfaces()` — CLAUDE.md.template +
   `skills/loom-plan-writer/SKILL.md` + every `agents/*.md` + `generate_stable_prefix()`'s
   generated text — so retiring a phrase from the canonical block is checked, not just remembered.
   `hooks/` is NOT covered by `guidance_surfaces()` — a retired phrase can still live on in a hook's
   own prose (e.g. `hooks/spawn-guard.sh`'s literal `PREAMBLE_LINE` constant), so a doctrine
   retirement must ALSO `rg` `hooks/` by hand.

**Counting the blocks is itself a drift surface.** This section is where an agent checks how many
there are, so an added block that is not added HERE reads as "does not exist" to the next reader.
When you add or retire a doctrine block, the table above is part of the change.

**Known tradeoff:** because the blocks must stay byte-identical, per-language examples cannot
live inside them. `BLOCK_A` carries one Rust example, so a blocked Python or Go subagent reads a
`cargo` example. The fix is to append language examples *after* the block as explicitly
surface-local guidance.

## Ambiguity = Fail Safe (Privilege Lookups From State Files)

Any hook or gate that grants a **relaxation** by reading a `.work/` state file must treat more
than one candidate match as ambiguous and refuse. Concretely: resolve by glob, count the
matches, and consult the file **only when exactly one exists**.

`glob | head -1` is not a lookup — it is a silent tie-break, and the tie is attacker-chosen when
filenames carry a sortable prefix. Cross-checking a field *inside* the file (`id:`) does not
help: whoever plants the decoy writes that field too. Only the count is trustworthy.

The matching test obligation: a fail-safe relaxation must be pinned by tests that assert the
**unsafe inputs are refused** (decoy present, wrong stage type, file missing). A test of the
granted direction alone leaves the relaxation free to regress silently.

## Token-Based Shell Command Classification

For a hook that must decide "does this Bash command run a project-wide build/test?", raw
substring matching is wrong (it blocks `rg "cargo test" doc/`) and naive word-splitting is
bypassable. The working shape:

1. Strip embedded content (heredoc bodies, `-m` message text) so a *mention* is not a match.
2. Normalise newlines to `;`, then pad separators — `;`, `|`, `(`, `)`, and the pair `&&`.
   Never pad a lone `&`: it would split `2>&1` and reopen a redirection bypass.
3. Split with `set -f` plus array splitting so quotes stay attached to their token.
4. Walk tokens tracking **command position** (line start, or after a separator, or after a
   prefix such as `env`/`time`/`xargs`) and **quote state**; only a runner in command position
   counts.
5. Skip redirection tokens when scanning for a positional argument — otherwise `2>&1` is read
   as a scope-narrowing filter.
6. **Allow unmatched commands.** A false block strands a subagent mid-task with no recourse;
   the gate is a guardrail, not a whitelist.

## Fail-Safe Direction for Destructive Sweeps (2026-08-08)

Any sweep that kills or deletes must resolve *uncertainty* toward inaction:

- **Cannot read the evidence ⇒ do not destroy.** `tmux/socket.rs`'s `socket_session_is_alive` returns
  `false` for an absent session file but **`true`** for one that exists and cannot be parsed — a file
  caught mid-write must not be read as "dead".
- **Cannot positively attribute ⇒ do not destroy.** Reap only resources provably owned by *this* work
  dir. Shared per-user namespaces (the tmux socket dir) make "no matching state file" match other
  checkouts' live resources.
- **Report what you skipped.** Unattributable resources are surfaced to the user, never silently
  killed and never silently ignored.

## Advisory Preflight: Do the Work, Report the Failure, Never Bail

`advisory_source_graph_preflight` (`commands/run/checks.rs:103-111`) is the second
instance of a shape worth copying, after `advisory_codex_lane_preflight`. The contract
is three rules and no more:

- it returns `()`, never a `Result`, so no caller can accidentally make it fatal;
- on failure it prints ONE `eprintln!` line with a stable prefix and swallows the error;
- it is idempotent and silent on the common path — `publish_source_graph`
  (`checks.rs:127-129`) early-returns when the layer for `HEAD` already exists.

Use it for derived state that IMPROVES a run but must never block one. The signature is
the enforcement: a function that cannot return an error cannot be made load-bearing by a
later caller who forgets it was optional.

# Untrusted Value Boundaries

> Enumerating every producer of a rendered field, not just the field, and why containment at one render site alone is not enough.

## Enumerate Every PRODUCER of a Rendered Field, Not Every Field

**What happened.** An agent audited the untrusted-data containment boundary personally,
traced the excerpt fencing, confirmed that chunk ANCHORS are normalized to
alphanumerics-plus-hyphens by `normalize_heading` (`fs/knowledge/chunker.rs:236`), and
concluded the boundary was closed. It was not. An independent reviewer found
`chunker.rs:73-77` takes the chunk id from
`frontmatter.id.clone().unwrap_or(derived_id)` for index 0 — so a knowledge file's YAML
frontmatter can set an **arbitrary multi-line id**, validated nowhere in `chunker.rs`,
`catalog.rs` or `ingest.rs`, and the brief renderer emitted it unfenced inside an inline
code span. A multi-line id closes the span and writes a markdown heading plus free prose
OUTSIDE the fence and outside the "quoted, NOT instructions" guard — into the signal file
the agent treats as its assignment.

**The misleading signal.** The auditor verified the DERIVED id path and the anchor path,
then generalised from two closed routes to "the identifier route is closed" — without
checking whether the id is always derived. One `unwrap_or` upstream turns a normalized
value into an arbitrary one, and the normalizer you read is then simply **not on the
path**.

**Prevention rule 1:** when auditing an escaping boundary, enumerate every PRODUCER of
each rendered field, not every field. Ask of each field "where can this value come from?"
and follow every arm, especially `unwrap_or`, `unwrap_or_default` and `Option` fallbacks.

**Prevention rule 2:** fencing the big obvious payload is the easy half. **The exploitable
half is always the small metadata rendered beside it** — ids, paths, anchors, summaries,
counts.

**Fix:** one `inline_safe()` flattener applied at the renderer (the containment boundary)
to id and pointer, with tests asserting the REFUSING direction.

## Containment At One Render Site Is Only As Good As the Set of Surfaces

**What happened.** After the fix above, the sanitized Knowledge Brief still pointed the
agent at an UNSANITIZED surface. `signals/format/brief.rs` fenced every excerpt and routed
ids, pointers and query through `inline_safe` — but the brief's own footer tells the agent
to run `loom knowledge context`, and that command's renderer did none of it:
`commands/knowledge/context.rs` `print_item` emitted `item.id`, `item.pointer.path` and
`item.summary` with a bare `println!`. Both fields are untrusted by the codebase's own
documentation. So an id carrying a newline plus `## SYSTEM INSTRUCTION ...` was correctly
flattened in the signal file and then rendered as REAL markdown structure in the agent's
tool output. `--json` was safe (serde escapes); default and `--explain` were not.

**Prevention rule:** when a fenced surface tells the agent to run a command, **that
command's output is part of the same trust boundary** and needs the same treatment. A
footer that advertises a second surface silently widens the set of surfaces the rule must
cover.

**Fix, and the shape worth copying:** `context/untrusted.rs` is now the ONE definition both
surfaces call — `MAX_INLINE_CHARS = 200`, backticks replaced with `ˋ` (U+02CB) — and its
module docstring states outright that a second copy would duplicate a security rule that
must never drift. Verified: `brief.rs:48/65/84/93` and
`commands/knowledge/context.rs:174/179/180/182` both route through it, and nothing else
defines a flattener.

## Why This Class Is Hard To See

The untrusted values here are not user input in any conventional sense — they are a chunk
id from a YAML frontmatter field, a filesystem path, and a markdown heading, all from files
in the project's own repository. Nothing about them reads as attacker-controlled, so the
usual instinct to sanitize does not fire. What makes them dangerous is the DESTINATION: a
signal file that an agent reads as its instructions.

**Rule:** classify by destination, not by origin. Any value rendered into agent-facing
prose is untrusted regardless of where it came from, because the blast radius is
"the agent does something else".

## A Glob Is Not a Containment Check (2026-08-27)

**What happened:** a fix under review validated an agent-influenceable file path with five
guards — absolute, under `$HOME/.claude/projects`, contains `/tool-results/`, is a regular file,
is not a symlink — and a thorough comment block explaining that the projects root is write-denied
to the sandboxed agent, so only the harness can author a file inside it. The reasoning was
correct. The predicate did not implement it:

```text
candidate:   $HOME/.claude/projects/../../../../tmp/claude-1000/tool-results/fake.txt
resolves to: /tmp/claude-1000/tool-results/fake.txt
RESULT: passes all five guards; marker read back successfully
```

`[[ "$p" == "$ROOT"/* ]]` is a pattern match on the RAW STRING. A `..` segment satisfies it
while the path resolves anywhere. `/tmp/claude*` and `$TMPDIR` are sandbox-writable by the stage
agent, so it could author the file and forge a trusted completion marker.

**Why it nearly shipped:** the comment argued the right invariant ("a file that genuinely resolves
under the write-denied root cannot be agent-authored") and the code tested a different, weaker
property (the string starts with that root). Careful reasoning attached to the wrong predicate
reads as thoroughness on review — the guards are numerous, the explanation is detailed, and
nothing looks omitted.

**Prevention:**

- **"Under directory D" is a claim about the RESOLVED path.** Either reject `..` segments outright
  (a genuine harness path never has one) or resolve first — and if you resolve, re-check and
  consume the RESOLVED value, or you have relocated a TOCTOU rather than closed it.
- **Reject the segment, not the substring.** `== *..*` also rejects the legitimate `foo..txt`.
  Match `../`, `/../`, `/..`, and a bare `..`; keep a positive test with a two-dot filename so the
  check cannot silently harden into a substring ban.
- **A `-L` test covers only the final component.** It is worth stating why intermediate symlinks
  are out of scope (here: the parent directories are themselves write-denied).
- **Test the attack, not the guard list.** The reproduction above is a one-line assertion; without
  it, five passing guards read as five layers of defence.

## Related

- `mistakes/completion-broker-credential.md` — the bridge this hardened, and why the
  write-denied projects root is the load-bearing premise.
- `mistakes/tests-that-cannot-fail.md` — the fix here needed tests asserting the refusing
  direction; an escaping test that only checks benign input passes forever.
- `architecture/context-retrieval.md` — where `inline_safe` sits in the pipeline.
- `patterns/doctrine-cross-surface.md` — the general problem of one rule across many
  surfaces.

## Display-Width Truncation Is Not a Containment Bound (2026-09-04)

A new instance of "classify by destination, not origin": `commands/status/ui/tui/ledger/text.rs`'s `truncate`/`cut_line`
budget against `Span::width()`, which is 0 for C0 controls (ESC included) and for the Cf/bidi/
zero-width set — so `used + character_width > budget` is never true for a zero-width character and
a 10,000-character ESC/ZWSP string passes a 16-cell column fully intact. The strings reaching those
cells are not all trusted: `stage.model`/`execution_models` come from the spawn ledger's
caller-controlled `.tool_input.model` (`hooks/spawn-guard.sh:334`), and `last_tool`/`last_activity`
come from heartbeat JSON (`commands/status/data/collector.rs:266-267`). A width-bounded renderer cannot do sanitization's
job — it does not even try, it just measures cells. The fix sanitizes once, at the collector boundary
that constructs the shared `StatusData` (`commands/status/data/sanitize.rs`, wired at
`commands/status/data/collector.rs:364`), using the same `inline_safe` from this file's main lesson — that one boundary
covers the wire, the static renderer, and the ledger TUI at once, where fixing each renderer
separately would not.

A second bug rode on the same code: `commands/status/data/execution_models.rs` deduped model names on the RAW ledger
string while flattening happened later, so `"sonnet<ZWSP>"` and `"sonnet "` passed dedup as distinct
and rendered as two identical-looking rows — and a trailing zero-width character could hide a
`-YYYYMMDD` date stamp from the suffix stripper. Fixed by calling `inline_safe` BEFORE
`normalize_model`, so the dedup key is the already-flattened display name. **Prevention:** when
flattening and deduping/normalizing both apply to the same value, flatten first — dedup/normalize
logic that runs on a not-yet-flattened string treats cosmetically-different byte sequences as
distinct keys.

## Security: Consolidated Findings

- **Socket permissions:** Created with default umask (world-accessible). Fix: `umask(0o077)` before bind.
- **PID handling:** `pid as i32` can overflow; raw `libc::kill` mishandles `EPERM`/`ESRCH`. Fix: use `nix::sys::signal::kill`.
- **Script injection:** AppleScript/XTerm strings not escaped. Fix: escape backslashes and quotes.
- **TOML injection:** `config.toml` via string formatting. Fix: use `toml::to_string_pretty`.
- **File locking TOCTOU:** `locked_write` truncated before lock. Fix: extracted `fs/locking.rs` with open-lock-truncate-write-flush.
- **State machine bypass:** `--force-unsafe` and recovery bypass skip validation. Fix: log all bypasses.

## String Handling: UTF-8 Truncation Panic

**Mistake:** Byte-level slicing `&s[..n]` panics on multi-byte UTF-8 characters.
**Fix:** Use `chars().take(n).collect::<String>()` for safe truncation.

## Byte-Index Slicing Panics on a Multi-Byte String From an Untrusted Source

**What happened:** `commands/status/data/execution_models.rs::normalize_model` stripped a trailing `-YYYYMMDD` stamp by
slicing a `&str` at a fixed byte offset (`len - 9`). The model name comes from `spawns.jsonl`, written
verbatim from a caller-controlled tool argument (`hooks/spawn-guard.sh::record_spawn`), and
`execution_models_for_stage` runs inside `collect_status_data`, which backs both `loom status` and
the daemon's broadcast — a single crafted spawn row with a multi-byte character near that offset
would panic the whole status subsystem.

**Why:** a fixed byte offset is not guaranteed to land on a UTF-8 character boundary; slicing at one
that doesn't panics at runtime, not compile time.

**Prevention:** strip a known-ASCII suffix/prefix with `rsplit_once`/`strip_suffix`/`char_indices`,
never a byte-offset slice, on any string that did not originate as a Rust literal you wrote yourself.

**Fix:** replaced the slice with `rsplit_once('-')`, which splits on a char boundary by construction.

## A Character-Class Allowlist Is Not a Path-Component Allowlist

**What happened:** `hooks/codex-forward.sh` and `hooks/spawn-guard.sh` both validate a stage id with
a character-class case (`case $stage_id in *[!A-Za-z0-9._-]* | "") return 0`), which ACCEPTS `.` and
`..` because both are made only of allowed characters. The resulting ledger path
`${work_dir}/subagents/${stage_id}` then resolves to the work dir itself for a `..` id — `chmod 700`
changes the work dir's own mode and the ledger row lands one level outside its per-stage directory.
The Rust reader (`commands/status/data/sanitize.rs::valid_stage_id`) gets this right by explicitly
rejecting `..`, so the two sides of the same boundary disagree, and the same check now exists in four
places at three different strengths (`commands/status/data/execution_models.rs:38`, `commands/memory/handlers/work_dir.rs:99`,
`hooks/codex-forward.sh:43`, `hooks/spawn-guard.sh:309`).

**Prevention:** when validating a value that becomes a path COMPONENT, reject `.` and `..` by name
explicitly — a charset check alone is not a path-component allowlist, whatever language it's written
in. For concerns.md: the four copies are a shared-helper candidate.

# Knowledge Cli Gaps

> Knowledge/memory CLI gaps: no delete-section, no blurb flag, CRLF, backlog

## Knowledge Signals Never Teach Tier-2 (2026-07-28)

`orchestrator/signals/` generates `loom knowledge update <tier-1-file>` guidance, but **no
prefix teaches the `category/slug` tier-2 form**. Verified functionally: tier-2 works
(`loom knowledge update patterns/lock-ordering` creates the file and `INDEX.md` picks it up on
the next knowledge write) — it is simply never advertised to an orchestrated knowledge stage.

Consequence: the hierarchy grows only through the interactive `bootstrap`/`gc` paths, not during
`loom run`. Not a defect in what landed; follow-up stage material.

## `loom knowledge` Has No Delete-Section Verb (2026-07-28)

`update` appends and `replace-section` replaces, but nothing removes a section. Consolidating
several tier-1 sections into one tier-2 topic therefore cannot be completed with the CLI alone —
the migration in this plan replaced the lead section with a summary and had to strip the
remaining N-1 headings with an external script. A `loom knowledge drop-section` (or a
`replace-section --delete`) would close the gap.

## Tier-2 Topic Blurbs Cannot Be Set From the CLI (2026-07-28)

A new topic is seeded with a fixed scaffold — a title derived from the slug and the blurb
"Topic notes for the `<category>` knowledge area" — and user content is appended _after_ it.
`scan_topics` harvests the **first** `#` and `>` lines for the INDEX.md table, so the generic
seeded blurb always wins and every topic reads identically in the index unless the file is edited
afterwards. The index's Blurb column is its main routing signal, so this directly costs
navigability. Wanted: a `--blurb` flag, or have the scaffold defer to a leading `>` line in the
supplied content.

## GC Flags Tier-1 Files for Section Extraction With No Oversized Sections (2026-07-31)

`analyze_gc_metrics` flags a tier-1 file whenever its **total** exceeds `DEFAULT_MAX_TIER1_LINES`
(250), independently of whether any individual section exceeds the section threshold. All six
tier-1 files here currently report `0 oversized sections` yet appear as extraction targets, and
the GC system prompt's first instruction is "Extract oversized tier-1 sections into tier-2 topic
files" — sections the analyzer itself says do not exist.

The agent is left to invent a split with no guidance on where the seams are, which is exactly the
condition under which a restructuring run drops content.

**Fix:** when a file is over budget but has no oversized section, say so in the prompt and ask for
a split proposal by topic cohesion instead of naming a section-extraction target that isn't there.

## `loom knowledge` Cannot Rename a Section Heading (2026-08-17, duplicate-heading half fixed 2026-08-19)

`loom knowledge replace-section <file> <heading> [content]` replaces a section's **body** and
keeps the existing heading line. The half of this concern about a DUPLICATE heading is now
fixed: `commands/knowledge/mod.rs::strip_repeated_heading` drops a `## <heading>` line repeated
at the top of the caller's content before splicing, so passing content with its own copy of the
heading no longer double-writes it.

**Still true:** `splice_section` (`fs/knowledge/dir.rs:278`) matches the EXISTING heading and
always re-emits that same heading text — there is no way to change the heading itself through
the CLI, so marking an entry resolved in the repo's `~~strikethrough~~ (RESOLVED date)`
convention still requires a direct file edit for the heading line, even though the body can now
be corrected in place.

**Fix:** either accept a `--heading <new>` flag, or match the OLD heading and re-emit whatever
heading line the content passed in.

## Two Fenced-Code-Block Models Disagree in `fs/knowledge/` (2026-08-26)

`splice.rs`'s `fence_mask` (the level-agnostic `replace-section` splicer) requires a closing
fence's run length to be at least as long as the opener's — CommonMark-correct. `chunker.rs`'s
`fence_marker` (used for retrieval chunking, `~:153-222`) closes on ANY line whose first
non-whitespace characters start with the same delimiter, regardless of run length. They disagree
on, e.g., a ` ``` ` line inside a ```` ```` ```` fence: `chunker.rs` treats it as closing the
fence early (so a `##` line just past it can be lexed as a heading) while `splice.rs` keeps
reading through to the real closer. Consequence: the span `loom knowledge context` reads a
heading from is not necessarily the span `replace-section` will overwrite. Should become one
shared scanner.

## `replace-section` Silently Converts CRLF to LF and Can Drop Trailing Blank Lines (2026-08-26)

`splice.rs::assemble_replacement` rebuilds the file from `base.lines()` (which strips both `\n`
and any preceding `\r`) joined back with plain `\n`, so a CRLF knowledge file is silently
rewritten to LF on any `replace-section` write. When the replaced section runs to EOF, any
trailing blank lines that followed the old section body are also dropped, since the tail beyond
the match is not copied forward. Harmless today — this tree's knowledge files are LF — but worth
knowing so a surprising whitespace-only diff after a `replace-section` call is explainable rather
than alarming.

## `loom knowledge update` Trims Stdin Content But Not Inline Content (2026-08-26)

`commands/knowledge/mod.rs::resolve_content` trims stdin input (`read_content_from_stdin` calls
`buffer.trim()`) but passes an inline CLI argument through untrimmed (`Some(c) => c`). An inline
`loom knowledge update <file> "<content with trailing blank lines>"` call therefore widens the gap
before the next appended section, while the same content piped via stdin would not. Minor, but the
two paths should agree.

## `loom memory` Is Unusable Without an Initialised `.work` (2026-08-11)

`loom memory note` exits non-zero with `.work directory not found. Run 'loom init' first.`
(`commands/memory/handlers/work_dir.rs`), and even past that gate the recording handlers require a
stage id from `--stage` or `LOOM_STAGE_ID` (e.g. `note()` in `commands/memory/handlers/record.rs`). Neither holds in
an interactive or ad-hoc session.

This collides head-on with doctrine: the mandatory subagent preamble orders every subagent to
record mistakes and decisions via `loom memory`, while auto-memory is prohibited whenever
`doc/loom/knowledge/` exists — which it does here. Agents are therefore ordered to record and
given no working way to do it, and the failure is silent from the orchestrator's point of view.
Three agents lost insights to this in a single session before it was noticed.

**Fixed in-tree 2026-08-11** (`commands/memory/handlers/work_dir.rs`): the four recording commands
(`note`, `decision`, `change`, `question`) now create `<repo_root>/.work/memory/` when cwd is
inside a git repo, and default the stage to the sentinel `ad-hoc`; `query`/`list`/`show` degrade
to exit 0 without creating anything. Outside a git repo the original error stands, so `.work` is
never scattered into arbitrary directories — see
[`find_repo_root_from_cwd` Returns `Some(cwd)` Outside Any Repo](../mistakes.md) for the trap that
guard exists to dodge.

**Still true until the built binary is installed:** a `loom` on PATH from before this change keeps
the old behaviour. When delegating outside a loom run against an older binary, tell subagents
explicitly that `loom memory` will fail, that auto-memory is still forbidden, and that they must
return insights in their final report for the orchestrator to record by hand.

**2026-09-10 note:** the single file `commands/memory/handlers.rs` no longer exists — it was split
into `commands/memory/handlers/{mod,work_dir,record,read,resolve,pending}.rs`; the error message
now reads "No loom workspace found. Run 'loom init' first." (`work_dir.rs::get_or_create_work_dir`),
same behaviour, updated wording.

## Tier-1 Knowledge Housekeeping Backlog

`loom knowledge check --strict` enforces 250 lines per tier-1 file, 40 lines per tier-1
section, and 12 KB for `INDEX.md` (`fs/knowledge/catalog/size.rs`). The remaining work is a
dedicated knowledge-reorganization project, not part of token-governor correctness.

- **All six tier-1 files remain oversized.** Their individual sections are compact; the
  overage is cumulative volume. Moving roughly 80-120 sections safely into tier-2 topics
  should be done file-by-file, preserving links and checking for duplicate headings.
- **`MissingSourceRef` remains the dominant finding.** Resolution needs the FULL path relative to a
  package's src root (e.g. commands/status/data/collector.rs, hooks/spawn-guard.sh) — a bare
  filename (collector.rs) or a partial suffix (ledger/legend.rs, ui/tui/app.rs) fails even when
  that suffix is unique in the tree; only the fully-qualified relative path resolves. The residual is
  mostly bare filenames, hook filenames written without their `hooks/` prefix, and genuinely stale
  citations that cannot be assigned to one package root safely. Canonicalize them to the full
  src-relative form; ambiguity must continue to fail closed.
- **Tier-2 topics with generic blurbs are unfixable from inside a stage session.** A stage session's
  `hooks/worktree-file-guard.sh` hook denies Edit/Write on any path under `doc/loom/knowledge/`, and
  there is no `loom knowledge` CLI verb for the blurb line specifically (only `update`, which appends,
  and `replace-section`, which needs an existing `#{2,6}` heading — the blurb is a bare `>` line under
  the H1). A knowledge-distill stage that creates a new tier-2 topic via
  `loom knowledge update <category>/<slug>` therefore cannot repair its own auto-scaffolded "Topic
  notes for the `<category>` knowledge area." blurb; that repair needs either a `--blurb` flag on
  `update`/a dedicated verb, or a direct file edit from an interactive (non-stage) session.
- **The generated index remains oversized.** This is low-priority navigation cleanup.
- **2026-09-04 reconfirmation (web-dashboard plan's knowledge-distill stage):** the backlog is at
  728 `MissingSourceRef`/oversized-file issues under `./doc/loom/knowledge` on a tree with none of
  this plan's own additions applied (verified against the unmodified HEAD tree via a temporary
  stash), essentially unchanged from prior counts. This plan's own new/edited knowledge content adds
  ZERO net new issues (verified: every bare filename this stage introduced was corrected to its
  fully-qualified src-relative path before completion). `loom knowledge check --strict` — the
  canonical knowledge-distill acceptance criterion per `skills/loom-plan-writer/SKILL.md` — therefore
  still fails on this tree for reasons entirely predating this plan; fixing the backlog itself needs
  the dedicated reorganization project named above, not a per-plan knowledge-distill stage. A stage
  hitting this should confirm (as here) that its own additions are clean, then treat the residual
  count as this pre-existing, already-tracked concern rather than attempting to clear it inline.

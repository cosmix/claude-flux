# Skill Catalog

> The two skill roots, why 53 skills live outside `~/.claude/skills`, and the install/hook-exemption hazards that came with the split.

## Skill Catalog: Two Roots, and Why the Split

Skills load from TWO roots, not one: `~/.claude/skills/` (9 CORE skills, named in
`skills/core-skills.txt`, one bare name per line, no trailing comments — Rust and bash readers
must agree byte-for-byte on the parsing rule: trim, skip blank/`#` lines) and
`~/.claude/loom-skill-catalog/` (the other ~53 skills). `loom/src/skills/index_catalog.rs` holds
the compiled-in core manifest (`include_str!` of `skills/core-skills.txt`), the two-root loader
`load_with_catalog`, and `skill_invocation()`, which renders the CORRECT form per root: a bare
`/loom-<name>` slash-form for a core skill (resolves directly), or
`Skill(skill="loom-skills", args="<name>")` for a catalogued one (per
`skills/index_catalog.rs::skill_invocation` — the catalog loader lives behind the `loom-skills`
skill, not a direct slash command). `loom/src/skills/install_layout.rs` reads
`~/.claude/loom-install.toml` and re-places skills into the correct root after a self-update.

**Both new modules are genuinely new files, not extensions of `index.rs`/`self_update/mod.rs`.**
Those two existing files are pinned at EXACT line counts in `loom/maintainability-baseline.txt`
(the ledger fails equally on growth AND shrinkage), and adding the catalog machinery inline would
have forced a ledger edit under a stage that did not own it. A new file under 400 lines needs no
ledger row at all — decide this BEFORE writing the implementation brief, not after the
maintainability test fails. `index.rs` itself only widened two visibility keywords
(`add_skill`/`parse_skill_file` -> `pub(super)`), which is net-zero on lines.

### Why 53 Skills Live Outside `~/.claude/skills`

Keeping the primary directory small is necessary but not sufficient — the split also exists
because of a hook interaction that would otherwise make most of the catalog unusable:
`hooks/read-guard.sh` is a GLOBAL `PreToolUse:Read` hook that warns above 400 lines and DENIES the
THIRD full read of a file past that limit. 22 catalogued skills exceed 400 lines (`loom-react`
1619 lines, `loom-istio` 1205) and the loader instructs the model to read a catalogued `SKILL.md`
IN FULL — without an exemption, rule 1 would argue the model into a partial (and useless) load,
and rule 2 would hard-block the third invocation of any popular oversized skill outright. The fix
was to exempt `SKILL.md` under BOTH skill roots from `read-guard.sh`'s line-count rules entirely,
rather than shrinking 22 skills or weakening the "read it in full" instruction. This was a
cross-stage collision found only at integration-verify: the guard-hooks stage's `read-guard.sh`
merged one commit before the skills-catalog stage, so no single stage's own tests could have
caught it — a lesson to drive a new large-file READ path against a real oversized example before
trusting the build.

### A Second, Independent Exemption Was Needed for the Same Reason

A catalogued `SKILL.md` is loaded via the **Read tool**, not the Skill tool (the loader's
`allowed-tools: [Read]`). `hooks/worktree-file-guard.sh` is a separate `PreToolUse` guard on
Read/Glob/Grep that blocks every path outside the worktree, so inside a worktree stage session
BOTH skill roots return exit 2 unless exempted — and `hooks/_read_discipline.sh` (the shared core
behind `read-guard.sh`/`poll-guard.sh`) needed the SAME exemption independently, since it is a
second, separate Read-class hook. Only one of the two guards was patched at first, leaving all 53
catalogued skills unreachable from a worktree session even after the read-guard fix landed.
**Rule: a feature that moves loading from the Skill tool to the Read tool must be re-checked
against every Read-class hook, not just the first one found.**

The worktree-file-guard exemption itself had a path-matching trap: a shell `case` glob written as
`*.claude/skills/*/SKILL.md` matches any directory ANYWHERE on disk whose name merely ends in
`.claude` (the leading `*` crosses `/`), and later a canonicalized `RESOLVED_PATH` (via
`realpath`) was compared against a non-canonicalized `$HOME`-built root — so a `HOME` that is
itself a symlink (autofs homes, `/home/x -> /data/x`, macOS `/Users`) silently re-broke the
exemption. Both bugs share one prevention rule: anchor a containment exemption to a root built
from `$HOME` (fail closed if `HOME` is unset) with the variable component constrained to ONE path
segment, and canonicalize BOTH sides of any path comparison with the same helper before diffing
them.

### Install and Migration Hazards (recorded, some pre-existing)

- **`cp -R "$dir/" dest/` copies the directory's CONTENTS on GNU coreutils but the DIRECTORY
  ITSELF on BSD/macOS `cp`** (`cp`'s own docs: a trailing `/` on the source copies contents on
  BSD). A glob like `"$src"/loom-*/` always yields a trailing slash, so this form passes every
  Linux test and silently splatters files into the destination root on macOS. Always write
  `cp -R "${dir%/}" dest/` instead (found in `install.sh:288`, `install_skills_from_source`).
- **Bare skill and agent names are user-owned.** `dev-install.sh` delegates to `install.sh`, so
  even local development installs must never derive `rust`, `software-engineer`, or another bare
  name from a `loom-*` entry and delete it as migration cleanup. Installation now replaces only
  Loom-prefixed destinations; legacy reference migration remains the explicit `loom repair`
  path. Behavior tests seed both bare and custom user entries and require them to survive.
- **`self_update/mod.rs::download_verify_and_extract_zip` backs up `dest` to `dest.bak`, extracts
  fresh, then DELETES the backup.** For `skills.zip`, `dest` is `~/.claude/skills`, so a
  self-update wipes any NON-loom skill a user keeps there. Pre-existing, out of the catalog
  stage's scope. If this function is ever touched: extract into a temp dir and merge, never
  rename/replace a directory loom does not exclusively own.
- **`repair.rs`'s `LOOM_SKILL_NAMES` is NOT an install manifest**, despite reading like one. Its
  only two call sites are the legacy unprefixed-to-prefixed `settings.json` reference migration
  (checking for `Skill(<bare-name>` and rewriting to `Skill(loom-name`) — it never installs,
  lists, or locates skill directories. Read every call site of a name list before assuming what
  it is for.

### Deliberately Cut: Domain Triggers on the `loom-skills` Loader

The catalog split's brief asked for `loom-skills` (the entry point for catalogued skills) to carry
~20 domain triggers (`rust`, `python`, `docker`, ...) "so `skill-trigger.sh` keeps matching." This
was cut: `loom skill-index` already scans the catalog directory directly, so a catalogued skill
matches on ITS OWN triggers regardless. Keeping the domain triggers on `loom-skills` too meant the
loader competed with the very skills it points at, accumulating score across every keyword a
prompt touched while each real skill scored only on its own — observed live, a prompt touching
rust+terraform returned `/loom-skills` as the third suggestion, DISPLACING a real match.

### Dead Re-exports Are Invisible to Clippy in a Library Crate

`skills/mod.rs` re-exported 8 names but only 3 (`load_with_catalog`, `skill_invocation`,
`apply_install_layout`) had any external caller. `pub` items in a lib crate are never
dead-code-warned, so `cargo clippy` misses an unused re-export entirely — this class of debt needs
a deliberate sweep (`rg` each re-exported name for callers outside its own module), not a linter.

## Component Architecture (loom/src/skills/)

Loads skill metadata from SKILL.md files across the two roots above, builds an inverted index of trigger keywords, and matches stage descriptions against it. Components: `types.rs` (`SkillMetadata`, `SkillMatch`), `matcher.rs` (keyword matching, phrase match = 2pts, word match = 1pt, threshold 2.0), `index.rs` (`SkillIndex`, `load_from_directory`, `match_skills` — visibility of `add_skill`/`parse_skill_file` widened to `pub(super)` for the catalog loader, otherwise unchanged), `index_catalog.rs` (the compiled-in core manifest via `include_str!` of `skills/core-skills.txt`, the two-root loader `load_with_catalog`, and `skill_invocation()` which renders the correct invocation form), `install_layout.rs` (reads `~/.claude/loom-install.toml` and re-places skills after a self-update). Up to 5 skill recommendations are embedded in agent signals.

Diagnosis Module (`loom/src/diagnosis/`): analyzes failed/blocked stages. `DiagnosisContext` collects `crash_report`, `log_tail`, `git_status`, `git_diff`. Generates a diagnostic signal for Claude Code investigation. CLI: `loom diagnose <stage-id>`.

Map Module (`loom/src/map/`): automated codebase analysis that populates knowledge files. Detectors: project type, dependencies, entry points, structure, conventions, concerns. Features: `--deep` (3-level depth + concerns), `--focus` (filter entry points), `--overwrite`. CLI: `loom map`.

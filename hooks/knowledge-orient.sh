#!/usr/bin/env bash
# knowledge-orient.sh - Claude Code SessionStart hook (GLOBAL - every
# repository, every session) that points a freshly started session at this
# checkout's curated knowledge index, doc/loom/knowledge/INDEX.md.
#
# Registered globally by fs/permissions/hooks/config.rs::build, unlike
# hooks/session-start.sh which is a WORKTREE-only hook wired up at loom stage
# creation time (loom/src/hooks/config.rs). Without this, an ordinary
# interactive session in a repository that keeps a knowledge tree has nothing
# telling it the tree exists - CLAUDE.md rule 12's knowledge-first doctrine
# only reaches a session that already knows to look.
#
# Input: JSON from stdin - {"source": "startup"/"resume"/"compact"/"clear", ...}
#
# Output: at most ONE JSON object on stdout -
#   {"hookSpecificOutput": {"hookEventName": "SessionStart", "additionalContext": "..."}}
# Every skip/failure path exits 0 with NO output, so a missing `jq`, a
# missing index, or a stage session never disturbs the session.
#
# Skipped entirely when:
#   - LOOM_STAGE_ID is set - a stage session reads its signal's Knowledge
#     Brief instead, and hooks/session-start.sh (worktree-only) handles its
#     own re-anchor on compact/resume.
#   - `jq` is not on PATH - nothing here can safely parse the input or build
#     the output without it.
#   - `.source` is "compact" or "resume" - an interrupted session already has
#     context loaded; re-anchoring on resume is session-start.sh's job, not
#     this hook's.
#   - no doc/loom/knowledge/INDEX.md is found walking up from $PWD before a
#     `.git` repository boundary.

set -euo pipefail
umask 077

# Read stdin JSON. Cross-platform timeout: gtimeout (macOS+coreutils),
# timeout (Linux), or plain cat - same pattern as session-start.sh.
if command -v gtimeout &>/dev/null; then
	INPUT_JSON=$(gtimeout 1 cat 2>/dev/null || true)
elif command -v timeout &>/dev/null; then
	INPUT_JSON=$(timeout 1 cat 2>/dev/null || true)
else
	INPUT_JSON=$(cat 2>/dev/null || true)
fi

# A stage session reads its signal's Knowledge Brief; hooks/session-start.sh
# (the worktree-only hook) handles its own re-anchor on compact/resume.
if [[ -n "${LOOM_STAGE_ID:-}" ]]; then
	exit 0
fi

if ! command -v jq &>/dev/null; then
	exit 0
fi

SOURCE=$(printf '%s' "$INPUT_JSON" | jq -r '.source // empty' 2>/dev/null || true)
if [[ "$SOURCE" == "compact" ]] || [[ "$SOURCE" == "resume" ]]; then
	exit 0
fi

# Walk upward from $PWD looking for doc/loom/knowledge/INDEX.md, stopping at
# the repository boundary (a `.git` entry). Pure bash directory tests, no
# subprocesses in the walk - this runs on every session start in every
# repository on the machine, so it must stay close to free.
INDEX_PATH=""
dir="$PWD"
while [[ -n "$dir" ]]; do
	if [[ -f "$dir/doc/loom/knowledge/INDEX.md" ]]; then
		INDEX_PATH="$dir/doc/loom/knowledge/INDEX.md"
		break
	fi
	if [[ -e "$dir/.git" ]]; then
		break
	fi
	dir="${dir%/*}"
done

if [[ -z "$INDEX_PATH" ]]; then
	exit 0
fi

# Byte count and row count for the nudge's parenthetical. A malformed/missing
# read of either degrades to "0" rather than aborting - this hook must never
# fail loudly over its own advisory numbers.
BYTES=$(wc -c <"$INDEX_PATH" 2>/dev/null | tr -d '[:space:]')
[[ -n "$BYTES" ]] || BYTES=0
ROWS=$(grep -c '^| \[' "$INDEX_PATH" 2>/dev/null || true)
[[ -n "$ROWS" ]] || ROWS=0

CTX="KNOWLEDGE-FIRST: this repository keeps curated knowledge under doc/loom/knowledge/ (INDEX.md: ${ROWS} entries, ${BYTES} bytes). Before exploring the tree, read ${INDEX_PATH}, then only the sections it points to: the section for your area in a tier-1 summary, and the tier-2 topics your task touches. Pull a specific question with loom knowledge context --query \"...\" --budget-tokens <n> (the matching sections come back quoted). Ask the source graph before opening files: loom map --outline <file>, loom map --find-all <symbol>, loom map --impact <symbol|path>."

jq -nc --arg ctx "$CTX" '{hookSpecificOutput: {hookEventName: "SessionStart", additionalContext: $ctx}}'
exit 0

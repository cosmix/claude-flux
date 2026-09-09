#!/usr/bin/env bash
# Shared read-discipline core for Read and Bash-side cat/head/tail/sed.
# Source _common.sh first. This library owns size limits, skip lists,
# outline fetching and shared decisions; _read_ledger.sh holds ledger helpers.
#
# Bash 3.2+ compatible (macOS default), same constraint as _common.sh: no
# associative arrays, no `${arr[-1]}`, no `${var,,}`. Every `${arr[@]}`
# expansion is guarded by a `${#arr[@]}` count check first - an EMPTY array
# expanded under `set -u` on bash 3.2 is a hard error (see
# prefer-modern-tools.sh's comment on this exact bug).

if [[ "${_LOOM_READ_DISCIPLINE_LOADED:-}" == "1" ]]; then
	return 0
fi
_LOOM_READ_DISCIPLINE_LOADED=1

source "$(dirname "$0")/_read_ledger.sh"

# READ_GUARD_LINE_LIMIT - above this many lines (per `wc -l`), an unbounded
# read of a file is redirected to `loom map --outline` instead of being
# allowed straight through. Ties to CLAUDE.md rule 17's 400-line file-size
# limit: a file this long already violates that limit, so reading it whole
# is reading something that should not exist at this size.
READ_GUARD_LINE_LIMIT=400

# _LOOM_VERIFY_RUNNER_BASENAMES - runner basenames poll-guard.sh's rule 2
# must NEVER count toward the repeated-command deny: the acceptance loop is
# SUPPOSED to rerun `cargo test` many times. This DUPLICATES the dispatch
# list in hooks/subagent-verify-guard.sh's check_command (that file's line
# 343) rather than sourcing it - a hand-transcribed copy that must be kept
# in sync with it by hand.
_LOOM_VERIFY_RUNNER_BASENAMES="cargo|pytest|tsc|eslint|go|npm|bun|pnpm|yarn|make"

# _loom_is_verify_runner_command - Return 0 when the already-tokenized
# command (global LOOM_TOKENS) invokes one of _LOOM_VERIFY_RUNNER_BASENAMES
# in command position.
_loom_is_verify_runner_command() {
	loom_tokens_invoke "$_LOOM_VERIFY_RUNNER_BASENAMES"
}

# _loom_read_skip_extension <path> - Return 0 when <path>'s extension is a
# binary/image format that a size/outline check makes no sense for.
_loom_read_skip_extension() {
	local path="$1" ext lower
	[[ "$path" == *.* ]] || return 1
	ext="${path##*.}"
	lower=$(printf '%s' "$ext" | tr '[:upper:]' '[:lower:]')
	case "$lower" in
	png | jpg | jpeg | gif | webp | bmp | ico | svg | pdf | zip | gz | tar | bz2 | xz | zst | \
		wasm | so | dylib | dll | exe | bin | o | a | class | jar | mp3 | mp4 | mov | avi | \
		ttf | otf | woff | woff2)
		return 0
		;;
	esac
	return 1
}

# _loom_is_tier1_knowledge_path <path> - Return 0 when <path> is a tier-1
# knowledge file: doc/loom/knowledge/<name>.md with NO further directory
# component. A tier-2 topic file (doc/loom/knowledge/<category>/<slug>.md) is
# excluded. INDEX.md matches this shape too, but loom_read_discipline_check
# checks _loom_is_knowledge_index_path FIRST and exempts it outright - see
# that function.
_loom_is_tier1_knowledge_path() {
	local path="$1" rest
	case "$path" in
	*doc/loom/knowledge/*) ;;
	*) return 1 ;;
	esac
	rest="${path##*doc/loom/knowledge/}"
	case "$rest" in
	*/*) return 1 ;;
	*.md) return 0 ;;
	esac
	return 1
}

# _loom_is_knowledge_index_path <path> - Return 0 when <path> ends in
# doc/loom/knowledge/INDEX.md (any prefix). INDEX.md is the sanctioned
# orientation read under CLAUDE.md rule 12 - hooks/knowledge-orient.sh's
# SessionStart nudge points every session at it directly - so warning on it
# here would contradict the doctrine this hook exists to enforce.
_loom_is_knowledge_index_path() {
	case "$1" in
	*/doc/loom/knowledge/INDEX.md | doc/loom/knowledge/INDEX.md) return 0 ;;
	esac
	return 1
}

# Skills are intentionally read in full, including catalogued skills over
# 400 lines. This exempts read-discipline checks, not filesystem containment.
# Accept both clients' standard roots, native .agents skills, and custom installs.
_loom_is_skill_md_path() {
	local path="$1" root
	case "$path" in
	*.claude/loom-skill-catalog/*/SKILL.md | *.claude/skills/*/SKILL.md | \
	*.codex/loom-skill-catalog/*/SKILL.md | *.codex/skills/*/SKILL.md | \
	*.agents/skills/*/SKILL.md) return 0 ;;
	esac
	root="${BASH_SOURCE[0]%/hooks/loom/*}"
	[[ "$root" == "${BASH_SOURCE[0]}" ]] && root=""
	for root in "$root" "${CODEX_HOME:-}"; do
		[[ -n "$root" ]] || continue
		case "$path" in
		"${root%/}"/skills/*/SKILL.md | "${root%/}"/loom-skill-catalog/*/SKILL.md) return 0 ;;
		esac
	done
	return 1
}

# _loom_file_line_count <path> - echo <path>'s line count via `wc -l`, or 0
# when it is not a regular file (a redirect onto a missing path is a hard
# error even under 2>/dev/null, and this runs on every unbounded Read).
_loom_file_line_count() {
	[[ -f "$1" ]] && wc -l <"$1" 2>/dev/null | tr -d '[:space:]' || printf '0'
}

# _loom_read_full_lines <path> - echo <path>'s line count for an UNBOUNDED
# ("full") read, computed ONCE so a caller never pays for `wc -l` twice:
# read-guard.sh and poll-guard.sh each call this a single time per read and
# pass the result into loom_read_discipline_check, which hands it on to rule
# 1 instead of re-deriving it. Skips the `wc -l` entirely for a binary/image
# extension (_loom_read_skip_extension) - rule 1 never fires for those
# anyway, so counting lines in a PNG or a PDF would stream the whole file
# for nothing.
_loom_read_full_lines() {
	local path="$1"
	if _loom_read_skip_extension "$path"; then
		printf '0'
		return 0
	fi
	_loom_file_line_count "$path"
}

# _loom_sanitize_agent_id <raw> - echo <raw> if it is a safe path component
# ([A-Za-z0-9._-] only, non-empty), else "main". Applied to every agent id
# AND every session id that becomes part of a ledger PATH - LOOM_SESSION_ID
# (a directory component) and the payload's own `.session_id` used as a
# fallback filename - so neither can ever escape the ledger directory. The
# payload's session_id is agent-controlled: an unsanitized
# "../../home/user/x" would walk `$TMPDIR/loom-reads` out to an arbitrary
# path, creating parent directories on the way.
_loom_sanitize_agent_id() {
	local raw="${1:-}"
	if [[ -z "$raw" ]] || [[ "$raw" =~ [^A-Za-z0-9._-] ]]; then
		printf 'main'
	else
		printf '%s' "$raw"
	fi
}

# _loom_ledger_file <kind> <agent_id> <fallback_session_id> - echo the TSV
# ledger path for one of loom's read-discipline counters.
#   kind: "reads" (read-guard.sh Task C, and poll-guard.sh's Task D rule 3)
#         or "polls" (poll-guard.sh's Task D rule 2).
# In a stage session (LOOM_WORK_DIR/LOOM_SESSION_ID/LOOM_STAGE_ID all set and
# LOOM_WORK_DIR a real directory): one file per agent -
#   ${LOOM_WORK_DIR}/hooks/<kind>/${LOOM_SESSION_ID}/<agent_id>.tsv
# Otherwise: a single per-session file under TMPDIR, keyed by the payload's
# own session_id (there is no per-agent directory to shard into outside a
# stage) -
#   ${TMPDIR:-/tmp}/loom-<kind>/<fallback_session_id>.tsv
# Both LOOM_SESSION_ID and fallback_session_id are run through
# _loom_sanitize_agent_id before use - see that function's comment.
_loom_ledger_file() {
	local kind="$1" agent_id="$2" fallback_sid="$3"
	if [[ -n "${LOOM_WORK_DIR:-}" && -n "${LOOM_SESSION_ID:-}" && -n "${LOOM_STAGE_ID:-}" && -d "${LOOM_WORK_DIR:-}" ]]; then
		printf '%s/hooks/%s/%s/%s.tsv' "$LOOM_WORK_DIR" "$kind" "$(_loom_sanitize_agent_id "$LOOM_SESSION_ID")" "$agent_id"
	else
		printf '%s/loom-%s/%s.tsv' "${TMPDIR:-/tmp}" "$kind" "$(_loom_sanitize_agent_id "${fallback_sid:-unknown}")"
	fi
}

# _loom_outline_covered_rows <path> - echo `loom map --outline <path>`'s
# symbol rows (matching `^[[:space:]]+L[0-9]+-L[0-9]+[[:space:]]`), or return
# 1 (nothing echoed) when the graph does not cover <path>, the command
# produced no output, or NEITHER gtimeout NOR timeout is on PATH. That last
# case is deliberate, not a fallback to a bare untimed call: a cold source
# graph would then block the Read tool call indefinitely, so with no way to
# bound it we skip `loom map` entirely and let the caller take the warn
# branch instead. STDERR is always discarded: inside a worktree `loom map`
# prints a "could not refresh the working-tree source graph" warning that
# must never reach a deny message.
_loom_outline_covered_rows() {
	local path="$1" output rows timeout_bin
	if command -v gtimeout &>/dev/null; then
		timeout_bin="gtimeout"
	elif command -v timeout &>/dev/null; then
		timeout_bin="timeout"
	else
		return 1
	fi
	output=$("$timeout_bin" 2 loom map --outline "$path" 2>/dev/null || true)
	[[ -n "$output" ]] || return 1
	rows=$(printf '%s\n' "$output" | grep -E '^[[:space:]]+L[0-9]+-L[0-9]+[[:space:]]' || true)
	[[ -n "$rows" ]] || return 1
	printf '%s' "$rows"
	return 0
}

# LOOM_HOOK_WARNS - accumulator for queued warning messages. Declared at
# source time so `set -u` never trips on an unset array before the first
# note.
LOOM_HOOK_WARNS=()

# loom_hook_note_warn <message> - Queue <message> to be joined with any
# other queued warnings and emitted as ONE LOOM_HOOK_WARN additionalContext
# JSON object by loom_hook_emit_warns. Never exits.
loom_hook_note_warn() {
	LOOM_HOOK_WARNS+=("$1")
	return 0
}

# loom_hook_emit_warns - If any warnings were queued, print them joined by
# " | " as a single LOOM_HOOK_WARN additionalContext JSON object (the shape
# prefer-modern-tools.sh uses) and exit 0. A no-op, WITHOUT exiting, when
# nothing was queued - the caller's own trailing `exit 0` covers that case.
loom_hook_emit_warns() {
	if ((${#LOOM_HOOK_WARNS[@]} == 0)); then
		return 0
	fi
	local joined="" msg
	for msg in "${LOOM_HOOK_WARNS[@]}"; do
		if [[ -z "$joined" ]]; then
			joined="$msg"
		else
			joined="${joined} | ${msg}"
		fi
	done
	jq -nc --arg ctx "LOOM_HOOK_WARN: ${joined}" \
		'{hookSpecificOutput: {hookEventName: "PreToolUse", additionalContext: $ctx}}'
	exit 0
}

# loom_hook_deny_or_warn <message> - Deny (stderr heredoc, exit 2) only when
# BOTH hold: loom_deny_enabled (the repo's [hooks] deny_enabled=true switch,
# _common.sh) AND LOOM_MAIN_AGENT_PID is set and a LIVE ancestor of this
# process (is_ancestor, _common.sh) - proof a real loom stage session is
# running above us right now. Both are required for the same reason
# spawn-guard.sh's ENFORCEMENT GATE requires them (spawn-guard.sh:66-86):
# LOOM_WORK_DIR is repo-stable, not per-session - loom/src/fs/permissions/
# settings.rs persists it in the settings `env` block on purpose, and that
# block overrides the process environment for EVERY session opened in the
# repo. Gating on the switch alone would hard-block an ordinary interactive
# session's Read/Bash calls the moment an operator sets deny_enabled
# repo-wide, with no orchestrator in the process tree to fix it and no env
# var to unset. A failed liveness check behaves exactly like the switch
# being off: queue the message as a warning and return 0. Do not drop this
# as "redundant" with loom_deny_enabled - they gate different things.
loom_hook_deny_or_warn() {
	local message="$1"
	local main_pid="${LOOM_MAIN_AGENT_PID:-}"
	if [[ -n "$main_pid" ]] && is_ancestor "$main_pid" && loom_deny_enabled; then
		cat >&2 <<EOF
⛔ BLOCKED: ${message}
EOF
		exit 2
	fi
	loom_hook_note_warn "$message"
	return 0
}

# _loom_read_discipline_large_unbounded <path> <lines> - Return 0 when
# <path> qualifies as rule 1's candidate: a regular file, not a
# skipped/binary extension, whose line count <lines> (computed ONCE by the
# caller via _loom_read_full_lines) exceeds READ_GUARD_LINE_LIMIT. The
# full-vs-range decision is the caller's job.
_loom_read_discipline_large_unbounded() {
	local path="$1" n="$2"
	[[ -f "$path" ]] || return 1
	_loom_read_skip_extension "$path" && return 1
	[[ -n "$n" ]] || return 1
	((n > READ_GUARD_LINE_LIMIT))
}

# _loom_read_discipline_verdict1 <path> <lines> - Compute rule 1's verdict
# WITHOUT emitting it: sets LOOM_RD_V1_KIND (none|warn|deny) and
# LOOM_RD_V1_MSG. <lines> is <path>'s line count, computed ONCE by the caller
# (never re-derived here with another `wc -l`). Deny when the graph covers
# <path> (comes with the outline inline), warn otherwise. The caller decides
# whether this is even a rule-1 candidate (large + unbounded) before calling
# this.
_loom_read_discipline_verdict1() {
	local path="$1" n="$2" rows
	LOOM_RD_V1_KIND="none"
	LOOM_RD_V1_MSG=""
	if rows=$(_loom_outline_covered_rows "$path"); then
		LOOM_RD_V1_KIND="deny"
		LOOM_RD_V1_MSG="${path} is ${n} lines. Outline: ${rows}
Read the ranges you need with offset/limit."
	else
		LOOM_RD_V1_KIND="warn"
		LOOM_RD_V1_MSG="${path} is ${n} lines and not covered by the source graph - find sections with \`rg -n \"^#\" ${path}\` then read a range with offset/limit."
	fi
	return 0
}

# _loom_read_discipline_verdict2 <path> <kind> <lines> <ledger> - Compute
# rule 2's verdict WITHOUT emitting it: sets LOOM_RD_V2_KIND (none|warn|deny)
# and LOOM_RD_V2_MSG, based on ledger rows recorded BEFORE this read. Returns
# "none" outright for a binary/image extension (_loom_read_skip_extension) -
# repeated reads of a PDF's pages or an image are not the re-reading rule 2
# exists to catch, and offset/limit-shaped advice makes no sense for either.
_loom_read_discipline_verdict2() {
	local path="$1" kind="$2" lines="$3" ledger="$4"
	LOOM_RD_V2_KIND="none"
	LOOM_RD_V2_MSG=""
	_loom_read_skip_extension "$path" && return 0
	if [[ "$kind" == "full" ]]; then
		_loom_reads_full_count_and_ts "$ledger" "$path"
		if ((LOOM_READS_FULL_COUNT == 1)); then
			LOOM_RD_V2_KIND="warn"
			LOOM_RD_V2_MSG="${path} read in full at ${LOOM_READS_FULL_FIRST_TS}; cite it or read a range"
		elif ((LOOM_READS_FULL_COUNT >= 2)); then
			LOOM_RD_V2_KIND="deny"
			LOOM_RD_V2_MSG="${path} has been read in full ${LOOM_READS_FULL_COUNT} times already - cite the earlier read or read a specific range with offset/limit."
		fi
	else
		local prior n
		prior=$(_loom_reads_range_count "$ledger" "$path" "$lines")
		n=$((prior + 1))
		if ((n >= 3)); then
			LOOM_RD_V2_KIND="warn"
			LOOM_RD_V2_MSG="range ${lines} of ${path} has been read ${n} times - cite the earlier read instead of re-reading it"
		fi
	fi
	return 0
}

# loom_read_discipline_check <path> <kind:full|range> <lines> <agent_id>
# <fallback_session_id> - the shared core for read-guard.sh's Task C Read
# checks and poll-guard.sh's Task D Bash-side cat/sed/head/tail checks. Same
# rules regardless of which tool performed the read:
#   0. A skill's SKILL.md (either skill root) is exempt outright, with no
#      warning at all - loading one whole is the intended way to use it, not
#      a read-discipline violation. Checked before rule 3, since a skill
#      path is never also a tier-1 knowledge path.
#   0b. doc/loom/knowledge/INDEX.md is exempt outright too, with no warning -
#      it is the sanctioned orientation read (CLAUDE.md rule 12;
#      hooks/knowledge-orient.sh's SessionStart nudge points at it directly).
#      Checked before rule 3, for the same reason as rule 0.
#   1. An unbounded ("full") read of a file over READ_GUARD_LINE_LIMIT lines
#      is redirected to `loom map --outline` (denied when covered, warned
#      when not).
#   2. A repeat full read, or 3+ identical range reads, of the same path is
#      warned (3rd+ full read is denied).
#   3. A tier-1 knowledge file (other than INDEX.md) read in a stage session
#      is warned, and OVERRIDES rules 1 and 2 outright (never denied).
#
# Rules 1 and 2 are both computed (verdict1/verdict2), then exactly ONE
# decision is emitted - a deny beats a warn, rule 1 wins a deny/deny tie (its
# outline is the more useful message). Comparing both is what lets rule 2
# eventually escalate to a deny on the Nth full read of a large file even
# though rule 1 has something to say every time too - otherwise rule 1 would
# talk forever and rule 2's repeat-read deny would never get a turn.
#
# Decisions are queued via loom_hook_note_warn/loom_hook_deny_or_warn, not
# emitted directly, so a caller evaluating several independent rules
# (poll-guard.sh) can still join every warning into one JSON object.
loom_read_discipline_check() {
	local path="$1" kind="$2" lines="$3" agent_id="$4" fallback_sid="$5"
	local ledger
	ledger=$(_loom_ledger_file "reads" "$agent_id" "$fallback_sid")

	if _loom_is_skill_md_path "$path"; then
		_loom_ledger_append "$ledger" "$path" "$kind" "$lines"
		return 0
	fi

	if _loom_is_knowledge_index_path "$path"; then
		_loom_ledger_append "$ledger" "$path" "$kind" "$lines"
		return 0
	fi

	if _loom_is_tier1_knowledge_path "$path" && [[ -n "${LOOM_STAGE_ID:-}" ]]; then
		loom_hook_note_warn "${path} is a tier-1 knowledge summary - pull the specific question instead: loom knowledge context --stage ${LOOM_STAGE_ID} --query \"...\""
		_loom_ledger_append "$ledger" "$path" "$kind" "$lines"
		return 0
	fi

	LOOM_RD_V1_KIND="none"
	LOOM_RD_V1_MSG=""
	if [[ "$kind" == "full" ]] && _loom_read_discipline_large_unbounded "$path" "$lines"; then
		_loom_read_discipline_verdict1 "$path" "$lines"
	fi
	_loom_read_discipline_verdict2 "$path" "$kind" "$lines" "$ledger"

	if [[ "$LOOM_RD_V1_KIND" == "deny" ]]; then
		loom_hook_deny_or_warn "$LOOM_RD_V1_MSG"
	elif [[ "$LOOM_RD_V2_KIND" == "deny" ]]; then
		loom_hook_deny_or_warn "$LOOM_RD_V2_MSG"
	elif [[ "$LOOM_RD_V1_KIND" == "warn" ]]; then
		loom_hook_note_warn "$LOOM_RD_V1_MSG"
	elif [[ "$LOOM_RD_V2_KIND" == "warn" ]]; then
		loom_hook_note_warn "$LOOM_RD_V2_MSG"
	fi

	_loom_ledger_append "$ledger" "$path" "$kind" "$lines"
	return 0
}

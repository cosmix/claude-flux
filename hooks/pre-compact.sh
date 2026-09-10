#!/usr/bin/env bash
# pre-compact.sh - Claude Code PreCompact hook for loom
#
# Implements block-then-allow pattern for context compaction:
# - First attempt: Blocks compaction, creates handoff, asks agent to dump context
# - Second attempt: Allows compaction after capturing updated state
#
# Input: JSON from stdin (session_id is forwarded to `loom hook pre-compact`,
# which resets session-scoped delivery suppression - see below - the rest of
# this script does not need it)
#
# Environment variables (set by loom worktree settings):
#   LOOM_STAGE_ID    - The stage being executed
#   LOOM_SESSION_ID  - The session ID
#   LOOM_WORK_DIR    - Path to the state directory (.loom/work, or the
#                      legacy .work for a workspace that already resolved
#                      to it)
#
# Exit codes:
#   0 = Allow compaction
#   2 = Block compaction (non-zero, non-1 to avoid hook failure)

set -euo pipefail

# Capture stdin (bounded by the same cross-platform timeout that used to just
# drain it) and hand the payload to `loom hook pre-compact` before anything
# else runs. That delegate resets THIS session's own delivery-suppression
# record so a post-compaction prompt is eligible for redelivery of context
# the compacted window may have lost (A.16/A.21,
# loom/src/commands/hook/pre_compact.rs). It must run BEFORE the env-var
# guard below and unconditionally in every session, loom-spawned or not — an
# ordinary non-loom session is exactly the case A.16 exists to serve, and
# this hook is registered globally regardless of LOOM_STAGE_ID. `loom hook
# pre-compact` is fail-open by contract, and `|| true` guards this call too:
# nothing here may ever block compaction over delivery-record bookkeeping.
if command -v gtimeout &>/dev/null; then
	STDIN_PAYLOAD=$(gtimeout 1 cat 2>/dev/null || true)
elif command -v timeout &>/dev/null; then
	STDIN_PAYLOAD=$(timeout 1 cat 2>/dev/null || true)
else
	STDIN_PAYLOAD=$(cat 2>/dev/null || true)
fi
if command -v loom &>/dev/null; then
	printf '%s' "$STDIN_PAYLOAD" | loom hook pre-compact >/dev/null 2>&1 || true
fi

# Validate required environment variables
if [[ -z "${LOOM_STAGE_ID:-}" ]] || [[ -z "${LOOM_SESSION_ID:-}" ]] || [[ -z "${LOOM_WORK_DIR:-}" ]]; then
	echo "Error: Missing required environment variables" >&2
	exit 1
fi

# Validate work directory exists and is accessible
if [[ ! -d "${LOOM_WORK_DIR}" ]]; then
	echo "Warning: Work directory does not exist: ${LOOM_WORK_DIR}" >&2
	exit 0 # Exit gracefully
fi

# Ensure hooks directory exists
HOOKS_DIR="${LOOM_WORK_DIR}/hooks"
mkdir -p "$HOOKS_DIR" 2>/dev/null || {
	echo "Warning: Cannot create hooks directory" >&2
	exit 0
}

# Log event to events.jsonl
EVENTS_FILE="${HOOKS_DIR}/events.jsonl"
TIMESTAMP=$(date -u +"%Y-%m-%dT%H:%M:%S.000Z")

# Check for compaction-pending flag file
PENDING_DIR="${LOOM_WORK_DIR}/compaction-pending"
PENDING_FLAG="${PENDING_DIR}/${LOOM_SESSION_ID}"

if [[ -f "$PENDING_FLAG" ]]; then
	# SECOND compaction attempt - flag exists, allow compaction
	rm -f "$PENDING_FLAG"

	# Create handoff (captures updated memory)
	HANDOFF_FILE=""
	if command -v loom &>/dev/null; then
		if HANDOFF_OUTPUT=$(loom handoff --stage "${LOOM_STAGE_ID}" --session "${LOOM_SESSION_ID}" --trigger precompact 2>&1); then
			HANDOFF_FILE=$(echo "$HANDOFF_OUTPUT" | grep -oE '[^/]+\.md$' || echo "")
		fi
	fi

	# Re-anchoring after compaction is handled by session-start.sh on the
	# SessionStart(source=compact) event — no recovery marker is needed here.

	# Build payload JSON
	if [[ -n "$HANDOFF_FILE" ]]; then
		PAYLOAD="{\"type\":\"PreCompact\",\"phase\":\"allow\",\"handoff_file\":\"${HANDOFF_FILE}\"}"
	else
		PAYLOAD="{\"type\":\"PreCompact\",\"phase\":\"allow\"}"
	fi

	cat >>"$EVENTS_FILE" <<EOF
{"timestamp":"${TIMESTAMP}","stage_id":"${LOOM_STAGE_ID}","session_id":"${LOOM_SESSION_ID}","event":"PreCompact","payload":${PAYLOAD}}
EOF

	exit 0
else
	# FIRST compaction attempt - block and capture state
	mkdir -p "$PENDING_DIR" 2>/dev/null || true
	touch "$PENDING_FLAG"

	# Create initial handoff
	HANDOFF_FILE=""
	if command -v loom &>/dev/null; then
		if HANDOFF_OUTPUT=$(loom handoff --stage "${LOOM_STAGE_ID}" --session "${LOOM_SESSION_ID}" --trigger precompact 2>&1); then
			HANDOFF_FILE=$(echo "$HANDOFF_OUTPUT" | grep -oE '[^/]+\.md$' || echo "")
		fi
	fi

	# Build payload JSON
	if [[ -n "$HANDOFF_FILE" ]]; then
		PAYLOAD="{\"type\":\"PreCompact\",\"phase\":\"block\",\"handoff_file\":\"${HANDOFF_FILE}\"}"
	else
		PAYLOAD="{\"type\":\"PreCompact\",\"phase\":\"block\"}"
	fi

	cat >>"$EVENTS_FILE" <<EOF
{"timestamp":"${TIMESTAMP}","stage_id":"${LOOM_STAGE_ID}","session_id":"${LOOM_SESSION_ID}","event":"PreCompact","payload":${PAYLOAD}}
EOF

	# Tell the agent where its working state already is, and what belongs in
	# `loom memory` instead: durable lessons, never task state. The handoff
	# just written above (path parsed into $HANDOFF_FILE) already carries the
	# working state - a memory entry duplicating it is procedural noise that
	# distillation would treat as durable knowledge.
	INTERCEPT_HANDOFF_LINE=""
	if [[ -n "$HANDOFF_FILE" ]]; then
		INTERCEPT_HANDOFF_LINE="Your working state is already captured in: ${HANDOFF_FILE}"
	else
		INTERCEPT_HANDOFF_LINE="Your working state is already captured in the handoff just written."
	fi

	cat >&2 <<INTERCEPT

CONTEXT COMPACTION INTERCEPTED
${INTERCEPT_HANDOFF_LINE}
Do NOT record task state to 'loom memory' - it is not read back after compaction.
Record only DURABLE lessons there (mistakes, decisions, gotchas), if any are worth keeping.
Continue work. Compaction will proceed on next cycle.

INTERCEPT

	exit 2
fi

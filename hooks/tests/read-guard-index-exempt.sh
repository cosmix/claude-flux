#!/usr/bin/env bash
# read-guard-index-exempt.sh - doc/loom/knowledge/INDEX.md is exempt from the
# tier-1 knowledge warning (rule 3 in hooks/_read_discipline.sh), since it is
# now the sanctioned orientation read (hooks/knowledge-orient.sh's
# SessionStart nudge points at it directly); every OTHER tier-1 knowledge
# file (e.g. architecture.md) must still warn.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
HOOK="$SCRIPT_DIR/../read-guard.sh"

TMP=$(mktemp -d "${TMPDIR:-/tmp}/loom-hooktest.XXXXXX")
trap 'rm -rf "$TMP"' EXIT

KNOWLEDGE_DIR="$TMP/doc/loom/knowledge"
mkdir -p "$KNOWLEDGE_DIR"
printf '# Knowledge Index\n\n| File |\n| --- |\n' >"$KNOWLEDGE_DIR/INDEX.md"
printf '# Architecture\n\nSome content.\n' >"$KNOWLEDGE_DIR/architecture.md"

# run_hook <file-path> - invoke read-guard.sh with LOOM_STAGE_ID=stage-x (a
# stage session) but no live loom main agent (LOOM_MAIN_AGENT_PID unset), so
# every decision below is a warning rather than a deny, and a private
# TMPDIR so the read ledger this run writes never touches the real one.
run_hook() {
	local file_path="$1"
	local input
	input=$(jq -nc --arg fp "$file_path" '{tool_name:"Read",tool_input:{file_path:$fp}}')
	printf '%s' "$input" |
		env -u LOOM_MAIN_AGENT_PID -u LOOM_HOOK_DEBUG -u COMMIT_FILTER_DEBUG \
			-u LOOM_WORK_DIR LOOM_STAGE_ID=stage-x TMPDIR="$TMP" bash "$HOOK"
}

# --- (a) INDEX.md must produce NO output -------------------------------------
set +e
OUTPUT_A=$(run_hook "$KNOWLEDGE_DIR/INDEX.md" 2>/dev/null)
CODE_A=$?
set -e

if [[ $CODE_A -ne 0 ]]; then
	echo "FAIL: (a) expected exit 0 reading INDEX.md, got $CODE_A"
	exit 1
fi
if [[ -n "$OUTPUT_A" ]]; then
	echo "FAIL: (a) reading INDEX.md produced output"
	echo "output: $OUTPUT_A"
	exit 1
fi

# --- (b) architecture.md must still produce the tier-1 warning ---------------
set +e
OUTPUT_B=$(run_hook "$KNOWLEDGE_DIR/architecture.md" 2>/dev/null)
CODE_B=$?
set -e

if [[ $CODE_B -ne 0 ]]; then
	echo "FAIL: (b) expected exit 0 reading architecture.md, got $CODE_B"
	exit 1
fi
if [[ -z "$OUTPUT_B" ]]; then
	echo "FAIL: (b) reading architecture.md produced no output - the tier-1 warning is gone"
	exit 1
fi
if ! echo "$OUTPUT_B" | jq -e '.hookSpecificOutput.additionalContext' >/dev/null 2>&1; then
	echo "FAIL: (b) architecture.md output has no additionalContext"
	echo "output: $OUTPUT_B"
	exit 1
fi
CTX_B=$(echo "$OUTPUT_B" | jq -r '.hookSpecificOutput.additionalContext')
if ! echo "$CTX_B" | grep -qF "tier-1 knowledge summary"; then
	echo "FAIL: (b) additionalContext does not mention the tier-1 knowledge warning"
	echo "additionalContext: $CTX_B"
	exit 1
fi

echo "PASS: INDEX.md is exempt from the tier-1 knowledge warning; other tier-1 files still warn"

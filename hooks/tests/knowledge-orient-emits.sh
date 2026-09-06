#!/usr/bin/env bash
# knowledge-orient-emits.sh - knowledge-orient.sh emits exactly one
# additionalContext JSON object pointing at doc/loom/knowledge/INDEX.md,
# including the absolute index path and the table's row count, when run from
# a subdirectory of a repository that has one.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
HOOK="$SCRIPT_DIR/../knowledge-orient.sh"

TMP=$(mktemp -d "${TMPDIR:-/tmp}/loom-hooktest.XXXXXX")
trap 'rm -rf "$TMP"' EXIT

REPO="$TMP/repo"
mkdir -p "$REPO"
(cd "$REPO" && git init -q)

KNOWLEDGE_DIR="$REPO/doc/loom/knowledge"
mkdir -p "$KNOWLEDGE_DIR"
cat >"$KNOWLEDGE_DIR/INDEX.md" <<'EOF'
# Knowledge Index

| File | Description |
| --- | --- |
| [architecture.md](architecture.md) | Architecture summary |
| [patterns.md](patterns.md) | Reusable patterns |
| [conventions.md](conventions.md) | Naming and style conventions |
EOF

SUBDIR="$REPO/src/nested"
mkdir -p "$SUBDIR"

OUTPUT=$(cd "$SUBDIR" && printf '%s' '{"source":"startup"}' |
	env -u LOOM_STAGE_ID -u LOOM_WORK_DIR -u LOOM_MAIN_AGENT_PID bash "$HOOK")

if [[ -z "$OUTPUT" ]]; then
	echo "FAIL: expected additionalContext output, got none"
	exit 1
fi

if [[ "$OUTPUT" == *$'\n'* ]]; then
	echo "FAIL: expected exactly one output line"
	echo "output: $OUTPUT"
	exit 1
fi

if ! printf '%s' "$OUTPUT" | jq -e '.hookSpecificOutput.additionalContext' >/dev/null 2>&1; then
	echo "FAIL: additionalContext not found in output"
	echo "output: $OUTPUT"
	exit 1
fi

CTX=$(printf '%s' "$OUTPUT" | jq -r '.hookSpecificOutput.additionalContext')

EXPECTED_INDEX_PATH="$KNOWLEDGE_DIR/INDEX.md"
if ! printf '%s' "$CTX" | grep -qF "$EXPECTED_INDEX_PATH"; then
	echo "FAIL: absolute index path '$EXPECTED_INDEX_PATH' not found in additionalContext"
	echo "additionalContext: $CTX"
	exit 1
fi

if ! printf '%s' "$CTX" | grep -qF "3 entries"; then
	echo "FAIL: '3 entries' not found in additionalContext"
	echo "additionalContext: $CTX"
	exit 1
fi

echo "PASS: knowledge-orient.sh emits additionalContext with the index path and row count"

#!/usr/bin/env bash
# knowledge-orient-silent.sh - knowledge-orient.sh must exit 0 with NO output
# on every skip path: no INDEX.md found anywhere, a stage session
# (LOOM_STAGE_ID set), a compact source, and an INDEX.md that only exists
# above the repository's `.git` boundary.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
HOOK="$SCRIPT_DIR/../knowledge-orient.sh"

TMP=$(mktemp -d "${TMPDIR:-/tmp}/loom-hooktest.XXXXXX")
trap 'rm -rf "$TMP"' EXIT

write_index() {
	local path="$1"
	mkdir -p "$(dirname "$path")"
	cat >"$path" <<'EOF'
# Knowledge Index

| File | Description |
| --- | --- |
| [architecture.md](architecture.md) | Architecture summary |
EOF
}

# assert_silent <label> <dir> <stdin-json> [extra env assignments...] -
# scrubs LOOM_STAGE_ID/LOOM_WORK_DIR/LOOM_MAIN_AGENT_PID, then invokes the
# hook from <dir> with <stdin-json> on stdin and fails if it produced ANY
# output.
assert_silent() {
	local label="$1" dir="$2" stdin_json="$3"
	shift 3
	local output
	output=$(cd "$dir" && printf '%s' "$stdin_json" |
		env -u LOOM_STAGE_ID -u LOOM_WORK_DIR -u LOOM_MAIN_AGENT_PID "$@" bash "$HOOK")
	if [[ -n "$output" ]]; then
		echo "FAIL: $label produced output"
		echo "output: $output"
		exit 1
	fi
}

# --- (a) no INDEX.md anywhere ------------------------------------------------
REPO_A="$TMP/repo-a"
mkdir -p "$REPO_A/sub"
(cd "$REPO_A" && git init -q)
assert_silent "(a) no INDEX.md" "$REPO_A/sub" '{"source":"startup"}'

# --- (b) INDEX.md present but LOOM_STAGE_ID set (a stage session) -----------
REPO_B="$TMP/repo-b"
mkdir -p "$REPO_B/sub"
(cd "$REPO_B" && git init -q)
write_index "$REPO_B/doc/loom/knowledge/INDEX.md"
assert_silent "(b) LOOM_STAGE_ID set" "$REPO_B/sub" '{"source":"startup"}' LOOM_STAGE_ID=test-stage

# --- (c) source is "compact" --------------------------------------------------
REPO_C="$TMP/repo-c"
mkdir -p "$REPO_C/sub"
(cd "$REPO_C" && git init -q)
write_index "$REPO_C/doc/loom/knowledge/INDEX.md"
assert_silent "(c) source=compact" "$REPO_C/sub" '{"source":"compact"}'

# --- (d) INDEX.md only exists ABOVE the .git repository boundary ------------
REPO_D_PARENT="$TMP/repo-d-parent"
REPO_D="$REPO_D_PARENT/repo"
mkdir -p "$REPO_D/sub"
(cd "$REPO_D" && git init -q)
write_index "$REPO_D_PARENT/doc/loom/knowledge/INDEX.md"
assert_silent "(d) INDEX.md above .git boundary" "$REPO_D/sub" '{"source":"startup"}'

echo "PASS: knowledge-orient.sh stays silent on every skip path"

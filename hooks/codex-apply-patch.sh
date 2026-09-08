#!/usr/bin/env bash
# Bridge Codex's apply_patch payload to Loom's Claude-shaped file guards and
# record edited paths for the source-graph overlay after a successful patch.

set -euo pipefail

MODE="${1:-}"
case "$MODE" in
pre | post) ;;
*) echo "usage: codex-apply-patch.sh <pre|post>" >&2; exit 2 ;;
esac

if ! command -v jq &>/dev/null; then
	if [[ "$MODE" == "pre" ]]; then
		echo "LOOM: blocked apply_patch because jq is unavailable" >&2
		exit 2
	fi
	exit 0
fi

if command -v gtimeout &>/dev/null; then
	INPUT_JSON=$(gtimeout 5 cat 2>/dev/null || true)
elif command -v timeout &>/dev/null; then
	INPUT_JSON=$(timeout 5 cat 2>/dev/null || true)
else
	INPUT_JSON=$(cat 2>/dev/null || true)
fi

TOOL_NAME=$(printf '%s' "$INPUT_JSON" | jq -r '.tool_name // empty' 2>/dev/null || true)
[[ "$TOOL_NAME" == "apply_patch" ]] || exit 0

PATCH=$(printf '%s' "$INPUT_JSON" | jq -r '.tool_input.command // empty' 2>/dev/null || true)
if [[ -z "$PATCH" ]]; then
	[[ "$MODE" == "post" ]] && exit 0
	echo "LOOM: blocked apply_patch because its patch payload was missing" >&2
	exit 2
fi

declare -a TARGETS=()

remember_target() {
	local path="$1"
	local existing
	[[ -n "$path" ]] || return 0
	for existing in "${TARGETS[@]}"; do
		[[ "$existing" == "$path" ]] && return 0
	done
	TARGETS+=("$path")
}

run_guards() {
	local synthetic_tool="$1"
	local path="$2"
	local payload guard
	payload=$(printf '%s' "$INPUT_JSON" |
		jq -c --arg tool "$synthetic_tool" --arg path "$path" \
		'.tool_name = $tool | .tool_input = {file_path: $path}')
	for guard in worktree-file-guard.sh credential-guard.sh plans-path-guard.sh stage-terminal-guard.sh; do
		printf '%s' "$payload" | "$(dirname "$0")/$guard"
	done
}

while IFS= read -r line; do
	case "$line" in
	"*** Add File: "*)
		path=${line#"*** Add File: "}
		remember_target "$path"
		[[ "$MODE" == "post" ]] || run_guards Write "$path"
		;;
	"*** Update File: "* | "*** Delete File: "*)
		path=${line#*: }
		remember_target "$path"
		[[ "$MODE" == "post" ]] || run_guards Edit "$path"
		;;
	"*** Move to: "*)
		path=${line#"*** Move to: "}
		remember_target "$path"
		[[ "$MODE" == "post" ]] || run_guards Write "$path"
		;;
	esac
done <<<"$PATCH"

if [[ ${#TARGETS[@]} -eq 0 ]]; then
	[[ "$MODE" == "post" ]] && exit 0
	echo "LOOM: blocked apply_patch because no target paths could be validated" >&2
	exit 2
fi

[[ "$MODE" == "post" ]] || exit 0
[[ -n "${LOOM_STAGE_ID:-}" ]] || exit 0
command -v loom &>/dev/null || exit 0

declare -a ARGS=()
for path in "${TARGETS[@]}"; do
	ARGS+=(--path "$path")
done

if command -v gtimeout &>/dev/null; then
	gtimeout 3 loom context record-edit --stage "$LOOM_STAGE_ID" "${ARGS[@]}" >/dev/null 2>&1 || true
elif command -v timeout &>/dev/null; then
	timeout 3 loom context record-edit --stage "$LOOM_STAGE_ID" "${ARGS[@]}" >/dev/null 2>&1 || true
else
	loom context record-edit --stage "$LOOM_STAGE_ID" "${ARGS[@]}" >/dev/null 2>&1 || true
fi

exit 0

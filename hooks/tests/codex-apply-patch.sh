#!/usr/bin/env bash
set -euo pipefail

SOURCE="$(dirname "$0")/../codex-apply-patch.sh"
TMP=$(mktemp -d "${TMPDIR:-/tmp}/loom-codex-hook.XXXXXX")
trap 'rm -rf "$TMP"' EXIT

HOOK_DIR="$TMP/hooks"
BIN_DIR="$TMP/bin"
LOG="$TMP/guards.log"
LOOM_LOG="$TMP/loom.log"
mkdir -p "$HOOK_DIR" "$BIN_DIR"
cp "$SOURCE" "$HOOK_DIR/codex-apply-patch.sh"

for guard in worktree-file-guard.sh credential-guard.sh plans-path-guard.sh stage-terminal-guard.sh; do
	printf '%s\n' \
		'#!/usr/bin/env bash' \
		'payload=$(cat)' \
		'printf "%s:%s\n" "$(printf "%s" "$payload" | jq -r .tool_name)" "$(printf "%s" "$payload" | jq -r .tool_input.file_path)" >>"$LOG"' \
		'[[ "$(printf "%s" "$payload" | jq -r .tool_input.file_path)" != forbidden ]]' \
		>"$HOOK_DIR/$guard"
	chmod +x "$HOOK_DIR/$guard"
done

printf '%s\n' \
	'#!/usr/bin/env bash' \
	'printf "%s\n" "$*" >"$LOOM_LOG"' \
	>"$BIN_DIR/loom"
chmod +x "$BIN_DIR/loom" "$HOOK_DIR/codex-apply-patch.sh"

payload=$(jq -nc --arg patch $'*** Begin Patch\n*** Add File: src/new.rs\n+new\n*** Update File: src/old.rs\n@@\n-old\n+new\n*** Move to: src/moved.rs\n*** End Patch' \
	'{tool_name:"apply_patch",tool_input:{command:$patch}}')

LOG="$LOG" printf '%s' "$payload" | LOG="$LOG" "$HOOK_DIR/codex-apply-patch.sh" pre
[[ $(wc -l <"$LOG") -eq 12 ]]
rg -qF 'Write:src/new.rs' "$LOG"
rg -qF 'Edit:src/old.rs' "$LOG"
rg -qF 'Write:src/moved.rs' "$LOG"

denied=$(jq -nc --arg patch $'*** Begin Patch\n*** Update File: forbidden\n@@\n-old\n+new\n*** End Patch' \
	'{tool_name:"apply_patch",tool_input:{command:$patch}}')
if LOG="$LOG" printf '%s' "$denied" | LOG="$LOG" "$HOOK_DIR/codex-apply-patch.sh" pre; then
	echo "FAIL: a denying file guard did not block apply_patch"
	exit 1
fi

PATH="$BIN_DIR:$PATH" LOOM_LOG="$LOOM_LOG" LOOM_STAGE_ID=stage-1 \
	printf '%s' "$payload" | PATH="$BIN_DIR:$PATH" LOOM_LOG="$LOOM_LOG" LOOM_STAGE_ID=stage-1 \
	"$HOOK_DIR/codex-apply-patch.sh" post
rg -qF 'context record-edit --stage stage-1 --path src/new.rs --path src/old.rs --path src/moved.rs' "$LOOM_LOG"

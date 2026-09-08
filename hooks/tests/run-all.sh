#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PASS=0
FAIL=0
ERRORS=()

run_test() {
    local name="$1"
    local script="$2"
    if output=$(bash "$script" 2>&1); then
        echo "  PASS: $name"
        ((PASS++)) || true
    else
        echo "  FAIL: $name"
        echo "    Output: $output"
        ((FAIL++)) || true
        ERRORS+=("$name")
    fi
}

echo "Running hook tests..."
run_test "prefer-modern-tools: grep triggers warning" "$SCRIPT_DIR/prefer-modern-tools-grep.sh"
run_test "prefer-modern-tools: find triggers warning" "$SCRIPT_DIR/prefer-modern-tools-find.sh"
run_test "prefer-modern-tools: rg has no warning" "$SCRIPT_DIR/prefer-modern-tools-rg.sh"
run_test "prefer-modern-tools: quoted grep no warning" "$SCRIPT_DIR/prefer-modern-tools-quoted.sh"
run_test "post-tool-use: heartbeat only, no tool event" "$SCRIPT_DIR/post-tool-use-tool-event.sh"
run_test "post-tool-use does not persist tool output" "$SCRIPT_DIR/post-tool-use-empty-output.sh"
run_test "loom-control-complete: exact verified route only" "$SCRIPT_DIR/loom-control-complete.sh"
run_test "loom-control-complete: tokenized pre-filter catches forged quoting, ignores prose" "$SCRIPT_DIR/loom-control-tokenized-prefilter.sh"
run_test "session-start: compact source emits re-anchor" "$SCRIPT_DIR/session-start-compact.sh"
run_test "session-end: records the SessionEnd reason from stdin" "$SCRIPT_DIR/session-end-reason.sh"
run_test "worktree-file-guard: malformed non-empty input is blocked" "$SCRIPT_DIR/worktree-file-guard-truncated-input.sh"
run_test "worktree-file-guard: MultiEdit honours the worktree boundary" "$SCRIPT_DIR/worktree-file-guard-multiedit.sh"
run_test "credential-guard: capability tokens blocked through the state-root symlink" "$SCRIPT_DIR/credential-guard-tokens.sh"
run_test "credential-guard: sandbox denyRead gates file tools, search roots above a denial run" "$SCRIPT_DIR/credential-guard-deny-read.sh"
run_test "credential-guard: a .. after a nonexistent component is normalized, not treated as unresolvable" "$SCRIPT_DIR/credential-guard-dotdot.sh"
run_test "plans-path-guard: blocks ~/.claude/plans write" "$SCRIPT_DIR/plans-path-guard-blocks-claude-plans.sh"
run_test "plans-path-guard: blocks ~/.claude/projects/*/plans edit" "$SCRIPT_DIR/plans-path-guard-blocks-project-plans.sh"
run_test "plans-path-guard: allows doc/plans and other .claude paths" "$SCRIPT_DIR/plans-path-guard-allows-doc-plans.sh"
run_test "plans-path-guard: blocks MultiEdit to .claude plans, allows doc/plans" "$SCRIPT_DIR/plans-path-guard-multiedit.sh"
run_test "codex-forward-guard: agent_type gate pins both shim types" "$SCRIPT_DIR/codex-forward-guard-agent-type.sh"
run_test "subagent-gate-payload: agent_type/transcript_path decide before the process-tree fallback" "$SCRIPT_DIR/subagent-gate-payload.sh"
run_test "subagent-start: ledger rows retain parent session identity" "$SCRIPT_DIR/subagent-start-ledger.sh"
run_test "codex-forward-guard: blocks Edit by a forwarder" "$SCRIPT_DIR/codex-forward-guard-blocks-edit.sh"
run_test "codex-forward-guard: companion Bash allowed, other Bash blocked" "$SCRIPT_DIR/codex-forward-guard-bash-companion-only.sh"
run_test "codex-forward-guard: plain subagents, main sessions, no-path all untouched" "$SCRIPT_DIR/codex-forward-guard-ignores-others.sh"
run_test "codex-forward: wrapper preserves task argv" "$SCRIPT_DIR/codex-forward-wrapper.sh"
run_test "codex apply_patch: file guards and source-graph edit recording" "$SCRIPT_DIR/codex-apply-patch.sh"
run_test "codex-forward-guard: quoted and escaped prompts round-trip" "$SCRIPT_DIR/codex-forward-guard-quoting.sh"
run_test "git-add-guard: quoted prose allowed, real args blocked" "$SCRIPT_DIR/git-add-guard-quoting.sh"
run_test "_common: token helpers scan argv values, not quoted prose" "$SCRIPT_DIR/common-token-helpers.sh"
run_test "commit-filter: quoted prose about git is allowed, real commits blocked" "$SCRIPT_DIR/commit-filter-quoted-payload.sh"
run_test "worktree-isolation: quoted prose paths allowed, real traversal blocked" "$SCRIPT_DIR/worktree-isolation-quoted-payload.sh"
run_test "prefer-modern-tools: grep/find inside a quoted payload is not a command" "$SCRIPT_DIR/prefer-modern-tools-quoted-payload.sh"
run_test "prefer-modern-tools: missing rg/fd allows grep/find with a warning" "$SCRIPT_DIR/prefer-modern-tools-missing-rg-fd.sh"
run_test "require-jq: blocking guard fails closed without jq" "$SCRIPT_DIR/require-jq-blocking.sh"
run_test "require-jq: advisory hook warns non-blockingly without jq" "$SCRIPT_DIR/require-jq-advisory.sh"
run_test "stage-terminal-guard: blocks completed/verified, allows others" "$SCRIPT_DIR/stage-terminal-guard.sh"
run_test "stage-terminal-guard: blocks MultiEdit once the stage is terminal" "$SCRIPT_DIR/stage-terminal-guard-multiedit.sh"
run_test "user-prompt-context: short prompt produces no output" "$SCRIPT_DIR/user-prompt-context-short-prompt.sh"
run_test "user-prompt-context: no LOOM_WORK_DIR exits silently" "$SCRIPT_DIR/user-prompt-context-no-workdir.sh"
run_test "user-prompt-context: malformed JSON fails open" "$SCRIPT_DIR/user-prompt-context-malformed-json.sh"
run_test "user-prompt-context: output is a single JSON object" "$SCRIPT_DIR/user-prompt-context-single-json-line.sh"
run_test "user-prompt-context: oversized delegate output is suppressed" "$SCRIPT_DIR/user-prompt-context-oversized-output.sh"
run_test "post-tool-use: records Write/Edit path via loom context record-edit" "$SCRIPT_DIR/post-tool-use-write-edit-record.sh"
run_test "post-tool-use: exits 0 when loom is absent from PATH" "$SCRIPT_DIR/post-tool-use-no-loom-binary.sh"
run_test "post-tool-use: symlinked heartbeat skips only the heartbeat write" "$SCRIPT_DIR/post-tool-use-symlinked-heartbeat.sh"
run_test "post-tool-use: records MultiEdit file_path via loom context record-edit" "$SCRIPT_DIR/post-tool-use-multiedit-record.sh"
run_test "post-tool-use: records NotebookEdit notebook_path via loom context record-edit" "$SCRIPT_DIR/post-tool-use-notebookedit-record.sh"
run_test "post-tool-use: canonical context-ceiling pair is cached and selected by role" "$SCRIPT_DIR/post-tool-use-ceiling-resolution.sh"
run_test "post-tool-use: context ceiling hard block fires on every call at 100%" "$SCRIPT_DIR/post-tool-use-ceiling-hard-block.sh"
run_test "post-tool-use: context ceiling 80% warning fires once per session" "$SCRIPT_DIR/post-tool-use-ceiling-warn-once.sh"
run_test "post-tool-use: subagent ceiling ignores the stage's own context_ceiling_tokens" "$SCRIPT_DIR/post-tool-use-subagent-ceiling.sh"
run_test "subagent-stop: heartbeat refresh is serialized and cannot roll parent tokens back" "$SCRIPT_DIR/subagent-stop-heartbeat-lock.sh"
run_test "heartbeat protocol: ownership, SessionStart lock, abandoned recovery, atomic JSON" "$SCRIPT_DIR/heartbeat-protocol.sh"
run_test "post-tool-use: resident-token arithmetic (last record wins, torn line survives)" "$SCRIPT_DIR/post-tool-use-resident-tokens.sh"
run_test "post-tool-use: heartbeat write survives a ceiling exit 2" "$SCRIPT_DIR/post-tool-use-ceiling-heartbeat-survives.sh"
run_test "post-tool-use: judge writes its own heartbeat, bypassing the ownership gate" "$SCRIPT_DIR/post-tool-use-judge-heartbeat.sh"
run_test "read-guard: offset/limit arithmetic injection is dead, well-formed reads unchanged" "$SCRIPT_DIR/read-guard-offset-injection.sh"
run_test "read-guard: INDEX.md is exempt from the tier-1 warning" "$SCRIPT_DIR/read-guard-index-exempt.sh"
run_test "session-start: heartbeat write escapes a quoted transcript_path via jq" "$SCRIPT_DIR/session-start-heartbeat-escaping.sh"
run_test "knowledge-orient: emits the INDEX.md pointer on startup" "$SCRIPT_DIR/knowledge-orient-emits.sh"
run_test "knowledge-orient: silent without an index, inside a stage, on compact, past the git root" "$SCRIPT_DIR/knowledge-orient-silent.sh"
run_test "post-tool-use: commit reminder is tokenized - heredoc body ignored, real commits fire" "$SCRIPT_DIR/post-tool-use-commit-reminder-tokenized.sh"

echo ""
echo "Results: $PASS passed, $FAIL failed"

if [[ $FAIL -gt 0 ]]; then
    echo "Failed tests: ${ERRORS[*]}"
    exit 1
fi

#!/usr/bin/env bash
# codex-forward-guard.sh - pin codex forwarding shims to one companion call
#
# A recognized forwarder may make only a direct invocation of Loom's installed
# argv-aware forwarding wrapper. The command is accepted only when its parsed
# argument shape is exact and it contains no unquoted shell operators.
# Missing classification metadata is rejected rather than silently disabling
# the policy.
#
# Input: JSON from stdin - {"tool_name": ..., "tool_input": ...,
#        "agent_type": ..., "transcript_path": ...}
# Exit codes: 0 = allow, 2 = block (also jq not installed - fail closed)

set -euo pipefail

source "$(dirname "$0")/_common.sh"
loom_require_jq "codex-forward-guard.sh"

if command -v gtimeout &>/dev/null; then
	INPUT_JSON=$(gtimeout 1 cat 2>/dev/null || true)
elif command -v timeout &>/dev/null; then
	INPUT_JSON=$(timeout 1 cat 2>/dev/null || true)
else
	INPUT_JSON=$(cat 2>/dev/null || true)
fi

TOOL_NAME=$(printf '%s' "$INPUT_JSON" | jq -r '.tool_name // empty' 2>/dev/null || true)
AGENT_TYPE=$(printf '%s' "$INPUT_JSON" | jq -r '.agent_type // empty' 2>/dev/null || true)
TRANSCRIPT_PATH=$(printf '%s' "$INPUT_JSON" | jq -r '.transcript_path // empty' 2>/dev/null || true)

block_forwarder() {
	local reason="$1"
	loom_debug "DEBUG: BLOCKED codex forwarder tool=$TOOL_NAME reason=$reason"
	cat >&2 <<EOF
⛔ BLOCKED: codex forwarding policy could not authorize this tool call.

Reason: $reason

The forwarding shim may make one direct Bash call of this form:
  ~/.claude/hooks/loom/codex-forward.sh task '<prompt>' --model gpt-5.6-terra --effort xhigh --write
The wrapper path may instead be written out in full as
  $HOME/.claude/hooks/loom/codex-forward.sh
Write that path expanded, exactly as shown - a literal \$HOME is rejected,
because an unquoted \$ is a forbidden shell metacharacter.

Shell operators, pipelines, redirections, command substitution, and any other
tool are forbidden. If forwarding fails, report the error and stop.
EOF
	exit 2
}

[[ -n "$TOOL_NAME" ]] || block_forwarder "tool_name metadata is missing"

parse_shell_words() {
	local input="$1" state=plain word="" char="" started=0 i
	PARSED_WORDS=()

	for ((i = 0; i < ${#input}; i++)); do
		char=${input:i:1}
		case "$state" in
		plain)
			case "$char" in
			' ')
				if [[ $started -eq 1 ]]; then
					PARSED_WORDS+=("$word")
					word=""
					started=0
				fi
				;;
			"'") state=single; started=1 ;;
			'"') state=double; started=1 ;;
			'\') state=escape; started=1 ;;
			$'\n' | $'\r' | $'\t' | $'\v' | $'\f' | ';' | '|' | '&' | '<' | '>' | '`' | '$' | '(' | ')' | '#' | '*' | '?' | '[' | ']' | '{' | '}') return 1 ;;
			*) word+="$char"; started=1 ;;
			esac
			;;
		single)
			if [[ "$char" == "'" ]]; then state=plain; else word+="$char"; fi
			;;
		double)
			case "$char" in
			'"') state=plain ;;
			'\') state=double_escape ;;
			$'\n' | $'\r' | $'\t' | $'\v' | $'\f' | '$' | '`') return 1 ;;
			*) word+="$char" ;;
			esac
			;;
		escape)
			case "$char" in $'\n' | $'\r' | $'\t' | $'\v' | $'\f') return 1 ;; esac
			word+="$char"
			state=plain
			;;
		double_escape)
			case "$char" in
			'"' | '\') word+="$char"; state=double ;;
			*) return 1 ;;
			esac
			;;
		esac
	done

	[[ "$state" == plain ]] || return 1
	if [[ $started -eq 1 ]]; then PARSED_WORDS+=("$word"); fi
}

is_exact_forward_command() {
	parse_shell_words "$1" || return 1
	[[ ${#PARSED_WORDS[@]} -eq 8 ]] || return 1
	if [[ -n "${HOME:-}" ]]; then
		[[ "${PARSED_WORDS[0]}" == "~/.claude/hooks/loom/codex-forward.sh" || "${PARSED_WORDS[0]}" == "${HOME}/.claude/hooks/loom/codex-forward.sh" ]] || return 1
	else
		[[ "${PARSED_WORDS[0]}" == "~/.claude/hooks/loom/codex-forward.sh" ]] || return 1
	fi
	[[ "${PARSED_WORDS[1]}" == task && -n "${PARSED_WORDS[2]}" ]] || return 1
	[[ "${PARSED_WORDS[3]}" == --model ]] || return 1
	case "${PARSED_WORDS[4]}" in gpt-6-astra | gpt-5.6-sol | gpt-5.6-terra | gpt-5.6-luna) ;; *) return 1 ;; esac
	[[ "${PARSED_WORDS[5]}" == --effort ]] || return 1
	case "${PARSED_WORDS[6]}" in low | medium | high | xhigh | max | ultra) ;; *) return 1 ;; esac
	[[ "${PARSED_WORDS[7]}" == --write ]]
}

# record_codex_task <model> <effort> - Append one row to
# $LOOM_WORK_DIR/subagents/<stage-id>/codex.jsonl recording the codex model
# and effort an AUTHORIZED forward is about to run with. Called from
# enforce_forwarder only after is_exact_forward_command has already
# succeeded, so a blocked command never reaches here and records nothing.
#
# This hook is a PreToolUse hook, so - like spawn-guard.sh's record_spawn -
# it runs OUTSIDE the stage session's Bash sandbox and can reach the state
# directory even though it is a symlink into the main repo from inside a
# worktree. codex-forward.sh itself cannot do this recording: it runs INSIDE
# that sandbox, where the append through the worktree's symlink is denied and
# silently swallowed.
#
# Write discipline mirrors spawn-guard.sh:305-348 (record_spawn) exactly:
# plain mkdir/redirection and never the loom CLI (the state directory is a
# SYMLINK inside a worktree and loom's safe-write opens roots O_NOFOLLOW), a
# symlinked target file is refused, every step is best-effort so a recording
# failure can never change the decision already made, and it never writes to
# stdout (this hook's stdout is hook protocol).
record_codex_task() {
	local model="$1" effort="$2"
	local work_dir="${LOOM_WORK_DIR:-}" stage_id="${LOOM_STAGE_ID:-}"
	[[ -n "$work_dir" && -n "$stage_id" ]] || return 0

	case "$stage_id" in
	*[!A-Za-z0-9._-]* | "" | "." | "..")
		return 0
		;;
	esac

	local dir="${work_dir}/subagents/${stage_id}"
	mkdir -p -m 700 "$dir" 2>/dev/null || return 0
	chmod 700 "$dir" 2>/dev/null || true

	local file="${dir}/codex.jsonl"
	if [[ -L "$file" ]]; then
		return 0
	fi

	local ts line
	ts=$(date -u +"%Y-%m-%dT%H:%M:%S.000Z") || return 0
	line=$(jq -nc \
		--arg ts "$ts" \
		--arg stage_id "$stage_id" \
		--arg session_id "${LOOM_SESSION_ID:-}" \
		--arg model "$model" \
		--arg effort "$effort" \
		'{ts: $ts, stage_id: $stage_id, session_id: $session_id, model: $model, effort: $effort}' \
		2>/dev/null) || return 0

	if [[ -n "$line" ]]; then
		{ printf '%s\n' "$line" >>"$file"; } 2>/dev/null || return 0
		chmod 600 "$file" 2>/dev/null || true
	fi
	return 0
}

enforce_forwarder() {
	[[ "$TOOL_NAME" == "Bash" ]] || block_forwarder "forwarders may use Bash only"
	local command
	command=$(printf '%s' "$INPUT_JSON" | jq -r '.tool_input.command // empty' 2>/dev/null || true)
	[[ -n "$command" ]] || block_forwarder "Bash command metadata is missing"
	is_exact_forward_command "$command" || block_forwarder "command is not an exact forwarding-wrapper invocation"
	record_codex_task "${PARSED_WORDS[4]}" "${PARSED_WORDS[6]}"
	exit 0
}

case "$AGENT_TYPE" in
loom-codex-forwarder | codex:codex-rescue) enforce_forwarder ;;
esac

# A hook payload without either authoritative agent type or transcript metadata
# cannot establish that the caller is not a forwarder.
if [[ -z "$AGENT_TYPE" && -z "$TRANSCRIPT_PATH" ]]; then
	block_forwarder "agent_type and transcript_path metadata are both missing"
fi

# A known non-forwarder type is authoritative and needs no transcript fallback.
if [[ -n "$AGENT_TYPE" ]]; then
	exit 0
fi

# Only subagent transcripts carry the sentinel fallback. Main-session paths do
# not qualify because they also contain the sentinel in Agent tool payloads.
case "$TRANSCRIPT_PATH" in
*/subagents/agent-*.jsonl) ;;
*) exit 0 ;;
esac

[[ -f "$TRANSCRIPT_PATH" && -r "$TRANSCRIPT_PATH" && ! -L "$TRANSCRIPT_PATH" ]] || block_forwarder "subagent transcript metadata is unreadable or unsafe"

if LC_ALL=C dd if="$TRANSCRIPT_PATH" bs=200000 count=1 2>/dev/null | grep -qF 'LOOM-CODEX-FORWARD-ONLY'; then
	enforce_forwarder
fi

exit 0

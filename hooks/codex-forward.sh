#!/usr/bin/env bash
# codex-forward.sh - trusted argv boundary for the codex companion runtime

set -euo pipefail

if [[ $# -ne 7 || "$1" != "task" || "$3" != "--model" || "$5" != "--effort" || "$7" != "--write" ]]; then
	printf '%s\n' \
		'Usage: codex-forward.sh task <prompt> --model <model> --effort <effort> --write' >&2
	exit 2
fi

prompt=$2
model=$4
effort=$6

case "$model" in
gpt-6-astra | gpt-5.6-sol | gpt-5.6-terra | gpt-5.6-luna) ;;
*)
	printf 'Unsupported forwarding model: %s\n' "$model" >&2
	exit 2
	;;
esac

case "$effort" in
low | medium | high | xhigh | max | ultra) ;;
*)
	printf 'Unsupported reasoning effort: %s\n' "$effort" >&2
	exit 2
	;;
esac

# The codex model/effort this forwarded task runs with is recorded by
# codex-forward-guard.sh, a PreToolUse hook that runs OUTSIDE the stage
# session's Bash sandbox. This wrapper runs INSIDE that sandbox, where an
# append to the main repo's `.loom/work/subagents/` through the worktree's
# symlink is denied and silently swallowed - so the guard, not this wrapper,
# owns that ledger row.

# codex reads AGENTS.md, never CLAUDE.md; loom install-assets writes ~/.codex/AGENTS.md as
# standing doctrine, but this preamble remains the per-task stage contract - the only place
# the codex lane's doctrine cannot be forgotten by an orchestrator writing a prompt, so it is
# prepended here on every forwarded task rather than left to each caller to remember.
preamble=$(cat <<'CODEX_PREAMBLE'
=== LOOM CONTEXT (prepended automatically; your task follows the TASK marker) ===

You are implementing one slice of a loom-orchestrated stage inside a git worktree. That
worktree is your boundary. An orchestrator verifies and commits your work; you do neither.

NAVIGATE WITH THE SOURCE GRAPH INSTEAD OF PAGING FILES.
Loom keeps a tree-sitter index of this repository. Each command below answers in well under a
second, never writes inside your worktree, and works from any directory in the tree. Use them
first, and open a file only once one of them has told you which lines matter:

  loom map --find-all <symbol>      every definition of a name: path, line, kind
  loom map --outline <file>         the symbols in a file, with line ranges and signatures
  loom map --impact <symbol|path>   what reaches it, with path confidence
  loom knowledge context --query "<question>" --budget-tokens 1500
                                    ranked project knowledge plus matching source
  rg -n '<pattern>' <path>          literal text search
  sed -n '<first>,<last>p' <file>   the exact lines a lookup pointed you at
  loom knowledge context prints the sections it matched, quoted; open the file it names only when you need more of it.

Two things to expect from these commands, neither of which is a failure. They may print
`warning: could not refresh ...` or `warning: failed to refresh the context cache ...`, because
the cache they try to refresh lives outside your sandbox - the command still answers from the
published index, so do not retry it and do not report it as a block. And that index reflects your
branch point, so it will not show edits you or another agent made during this session; read a file
directly when you have already changed it.

DO NOT read CLAUDE.md. It instructs a different agent; nothing in it is addressed to you.
DO NOT read doc/loom/knowledge/ file by file. It is a ~200k-token corpus, and
`loom knowledge context --query` is how you query it. Ask it a question; do not read the library.

WRITE ONLY THE FILES YOUR TASK ASSIGNS YOU. Everything else in the tree is read-only to you.
NEVER write anything under .work/ or .loom/ - .loom/work/ (or the legacy .work/) is a symlink to
state shared with other running stages, and the rest of .loom/ is orchestrator-owned spool/cache
data.
NEVER run git: not add, not commit, not checkout, not stash, not restore.
DO NOT VERIFY. No full build, no test suite, no linter, no formatter, no type-checker, and never
a repeated or looping check. At most ONE narrowly-scoped check over the files you changed, run
once; skip it if you are unsure. The orchestrator compiles, tests, lints, and fixes.

FINISH BY REPORTING: files changed, assumptions you made, anything you could not resolve.

=== TASK ===
CODEX_PREAMBLE
)

task="${preamble}

${prompt}"

# macOS refuses to apply a second Seatbelt profile to an already-sandboxed
# process. Inside a stage session's Bash sandbox every command codex wraps in
# /usr/bin/sandbox-exec (its workspace-write AND read-only modes both do) dies
# with `sandbox-exec: sandbox_apply: Operation not permitted` while codex and
# the companion still exit 0. The companion hardcodes workspace-write and
# exposes no override, so when nesting is refused the wrapper calls
# `codex exec` directly with --sandbox danger-full-access: the outer sandbox
# (worktree + granted paths, strict domain allowlist) stays the boundary, the
# same containment any other subagent's Bash call has. PATH lookup on purpose:
# the probe only chooses a lane, and the tests stub it. On Linux there is no
# sandbox-exec, so the companion path (codex's own bubblewrap nested inside the
# stage sandbox) is unchanged. The probe is a heuristic: it asks whether ANY
# profile can be applied, which is the refusal macOS produces; a host that
# accepted this trivial profile yet rejected codex's own would still fall
# through to the companion path and fail silently as before.
nested_seatbelt_refused() {
	command -v sandbox-exec >/dev/null 2>&1 || return 1
	! sandbox-exec -p '(version 1)(allow default)' /usr/bin/true >/dev/null 2>&1
}

# newest-first, capped, via -nt insertion - no ls|head parsing
print_newest() {
	local cap=$1
	shift
	local newest=()
	local f i inserted
	for f in "$@"; do
		inserted=false
		for i in "${!newest[@]}"; do
			if [[ "$f" -nt "${newest[$i]}" ]]; then
				newest=("${newest[@]:0:$i}" "$f" "${newest[@]:$i}")
				inserted=true
				break
			fi
		done
		if [[ "$inserted" == false ]]; then
			newest+=("$f")
		fi
		if [[ ${#newest[@]} -gt $cap ]]; then
			newest=("${newest[@]:0:$cap}")
		fi
	done
	if [[ ${#newest[@]} -gt 0 ]]; then
		printf '%s\n' "${newest[@]}"
	fi
}

if nested_seatbelt_refused; then
	printf 'note: the outer sandbox refuses a nested Seatbelt profile; running codex exec with --sandbox danger-full-access (the outer sandbox is the boundary)\n' >&2
	status=0
	# stdin is closed deliberately: `codex exec` treats an open stdin as extra
	# prompt input ("Reading additional input from stdin...") and blocks until
	# EOF. Under the Bash tool stdin is already at EOF, so the hang only shows
	# on a caller that keeps it open - which is exactly the caller that cannot
	# report why it stalled. The companion lane needs no such redirect: it takes
	# the prompt positionally and only falls back to stdin when that is empty,
	# and it gives its own child pipes rather than this process's stdin.
	codex exec --sandbox danger-full-access --skip-git-repo-check \
		--model "$model" -c "model_reasoning_effort=\"$effort\"" -- "$task" </dev/null || status=$?
	printf '\n--- LOOM-CODEX-EVIDENCE ---\n'
	printf 'exit: %s\n' "$status"
	printf 'mode: direct (codex exec --sandbox danger-full-access; nested Seatbelt refused)\n'
	codex_home=${CODEX_HOME:-${HOME}/.codex}
	shopt -s nullglob
	rollouts=("$codex_home"/sessions/*/*/*/rollout-*.jsonl)
	shopt -u nullglob
	if [[ ${#rollouts[@]} -eq 0 ]]; then
		printf 'session: none found\n'
	else
		printf 'session: %s\n' "$(print_newest 1 "${rollouts[@]}")"
	fi
	exit "$status"
fi

versions_dir=${HOME:?HOME is required}/.claude/plugins/cache/openai-codex/codex
shopt -s nullglob
candidates=("$versions_dir"/*/scripts/codex-companion.mjs)
shopt -u nullglob

if [[ ${#candidates[@]} -eq 0 ]]; then
	printf 'codex-companion.mjs not found under %s\n' "$versions_dir" >&2
	exit 1
fi

companion=${candidates[0]}
for candidate in "${candidates[@]:1}"; do
	if [[ "$candidate" > "$companion" ]]; then
		companion=$candidate
	fi
done

if [[ ! -f "$companion" || -L "$companion" ]]; then
	printf 'Refusing unsafe companion path: %s\n' "$companion" >&2
	exit 1
fi

# The companion derives its job-state root from CLAUDE_PLUGIN_DATA:
# `stateRoot = $CLAUDE_PLUGIN_DATA/state` (the plugin's scripts/lib/state.mjs).
# Claude Code points that at ~/.claude/plugins/data/<plugin>, and some sandbox
# configurations deny writes anywhere under ~/.claude/plugins — the job-record
# mkdir then fails with EPERM before any model call, which reads as a codex or
# auth failure but is neither. A `sandbox.filesystem.allowWrite` grant does not
# help: it is the deny on the parent that wins.
#
# ~/.codex is already granted to this lane (CODEX_SANDBOX_WRITE_PATHS in
# loom/src/codex.rs), so redirect there — but ONLY when the configured root is
# genuinely unwritable. Machines where the default works keep it, so the
# plugin's own /codex:status and /codex:result keep finding their records where
# they expect them.
if [[ -n "${CLAUDE_PLUGIN_DATA:-}" ]] && ! mkdir -p "${CLAUDE_PLUGIN_DATA}/state" 2>/dev/null; then
	CLAUDE_PLUGIN_DATA="${HOME}/.codex/plugin-data"
	export CLAUDE_PLUGIN_DATA
	mkdir -p "${CLAUDE_PLUGIN_DATA}/state" 2>/dev/null || true
	printf 'note: plugin data root not writable; codex state redirected to %s\n' \
		"$CLAUDE_PLUGIN_DATA" >&2
fi

status=0
node "$companion" task "$task" --write --model "$model" --effort "$effort" || status=$?

printf '\n--- LOOM-CODEX-EVIDENCE ---\n'
printf 'exit: %s\n' "$status"
printf 'mode: companion\n'

state_root=${CLAUDE_PLUGIN_DATA:-${HOME}/.claude/plugins/data/codex-openai-codex}/state
shopt -s nullglob
job_files=("$state_root"/*/jobs/*.json)
shopt -u nullglob

if [[ ${#job_files[@]} -eq 0 ]]; then
	printf 'jobs: none found\n'
else
	print_newest 3 "${job_files[@]}"
fi

exit "$status"

# Shell Command Matchers

> Separators that never become tokens, forgeable glob lookups, env leakage in hook tests, and three Bash traps.

## Token-Based Shell Matchers: Separators That Never Become Tokens (SYSTEMIC, 2026-07-28)

**What happened:** `hooks/subagent-verify-guard.sh` classifies a Bash command by splitting it
into tokens and deciding whether a "runner" token sits in command position. Three separate
bypass classes shipped and were fixed across two stages, each the same root cause.

**Why:** IFS word-splitting only breaks on whitespace, so anything _glued_ to a neighbour never
becomes its own token.

| Bypass class                        | Example that slipped through                                                                                          | Why                                                                                                              |
| ----------------------------------- | --------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| Redirection read as an argument     | `cargo test 2>&1 \| tail -50`                                                                                         | `2>&1` hit the positional arm and looked like a test _filter_, making a project-wide run look scoped             |
| Newlines eaten by splitting         | `cd loom` + newline + `cargo test`                                                                                    | the newline never became a token, so command position never reset and every line after the first was unreachable |
| Glued metacharacters (13 confirmed) | `echo hi; cargo test`, `echo hi&&cargo test`, `true\|\|cargo build`, `{ cargo test; }`, every `if`/`while`/`for` body | a separator with no surrounding space is absorbed into the adjacent token (`test;` is not `test`)                |

**Prevention:** the fix for one of these is not the fix for the others, and a half-applied
normalizer looks correct. For any hook that classifies shell commands by tokenising:

1. Normalise newlines to `;` _before_ splitting.
2. Pad `;`, `|`, `(`, `)` and the **pair** `&&` — never a lone `&`, which would split `2>&1`
   into `2>` and `1` and reopen the redirection bypass.
3. Skip redirection tokens explicitly; they are not positional arguments.
4. Track quote state across tokens so a quoted mention (`rg "cargo test" doc/`) stays inert.
5. Probe every separator **both glued and spaced, in both directions**, and feed the matcher
   the repo's own documented command forms — the piped `2>&1 | tail -50` shape that global
   CLAUDE.md Rule 14 mandates is the single most likely real-world bypass.

**Fix:** normalisation lives in one place in `hooks/subagent-verify-guard.sh`; regression cases
are table-driven in `loom/tests/integration/hooks_subagent_verify_guard_cases.rs`.

## Glob + `head -1` Is Forgeable in a Security Gate (2026-07-28)

**What happened:** the integration-verify carve-out resolved its stage file with
`ls WORK_DIR/stages/*-<stage-id>.md | head -1`. Stage files carry a numeric prefix, so a planted
`00-<stage-id>.md` declaring `stage_type: integration-verify` beat the real `02-<stage-id>.md`
lexicographically and granted a full-suite carve-out to an ordinary stage. Reproduced: exit 0
where it must be 2.

**Why:** `head -1` silently _picks a winner_ among duplicates instead of reporting ambiguity.

**Prevention:** the obvious fix — prefer the file whose `id:` field agrees — does **not** work,
because whoever plants the decoy also writes its `id:`. The durable rule for any hook that
grants a privilege by reading a `.work` file: **more than one glob match means ambiguous; fail
safe and do not grant the relaxation.** Consult the file only when exactly one match exists.

**Fix:** ambiguity check in `hooks/subagent-verify-guard.sh`; refusal directions pinned in
`loom/tests/integration/hooks_subagent_verify_guard_carveout.rs`.

## A Fail-Safe Fix Needs a Test That Asserts the REFUSAL (2026-07-28)

**What happened:** the carve-out above shipped with a test for the _granted_ direction only.
Reverting the ambiguity check from `-eq 1` back to `-ge 1` left the entire suite green.

**Prevention:** when a fix is "we made this fail safe", a happy-path test proves nothing. The
regression test must assert that the **unsafe input is refused** — decoy, wrong stage type, and
missing file all belong in the table.

## Hook Tests Must Scrub the Env the Hook Gates On (2026-07-28)

**What happened:** a helper spawned simulated process trees with `Command::new("bash")` without
clearing the inherited environment, so `LOOM_STAGE_ID` / `LOOM_WORK_DIR` leaked from the outer
test process into every simulated tree. Inside an integration-verify stage session this poisons
the carve-out check: **21 of 32 "must be blocked" cases silently flip to false passes.**

**Prevention:** any test that spawns child processes to simulate a hook's runtime context must
`.env_remove()` every variable the hook reads for gating or carve-out decisions, and apply
intentional `extra_env` **after** the removal so opt-in cases still work.

## `$TMPDIR` Contains "claude" — It Breaks Process-Tree Simulation (2026-07-28)

**What happened:** in this harness `$TMPDIR` is `/tmp/claude-1000`, so `TempDir::new()` puts
every spawned test process's cmdline under a path containing `claude`. The name-based subagent
detector in `hooks/_common.sh` then matches even for non-claude-named scripts, and the hook
falsely fires against itself.

**Fix:** build the temp tree under a claude-free path — `tempfile::Builder::new().tempdir_in()`
rooted at `CARGO_MANIFEST_DIR/target/` — for any test that exercises `loom_is_subagent`.

## Bash: `&` in a `${var//pat/rep}` Replacement Expands to the Match (2026-07-28)

**What happened:** `${S//&&/ && }` on `echo hi&&cargo test` yields `echo hi &&&& cargo test`,
not the intended padding — `&` in the replacement means "the matched text", exactly like `sed`.
The hook then matched nothing, so the fix silently did nothing while reading as correct.

**Fix:** escape it — `${S//&&/ \&\& }`. **Detection:** after any `${//}` whose replacement
contains `&`, `printf` the result once before trusting it; the symptom is a substitution that
appears to run while downstream matching behaves as if unchanged.

## Testing `errexit` Inside `||` Disables the Very Thing Under Test (2026-07-28)

**What happened:** `( set -e; ((count++)); echo ok ) || echo aborted` was used to prove the
`((count++))` errexit trap survived. It cannot fail: a subshell placed in a `||` list has
`errexit` suppressed inside it.

**Prevention:** to test `errexit` behaviour, run the snippet as its own `bash -c` and judge it
by exit status. Never wrap it in `||`, `&&`, or `if` — those are the exact contexts that
disable `set -e`.

## `\b` After an Escaped Period Is Not a Word Boundary in ERE (2026-07-28)

**What happened:** `commit-filter.sh`'s subagent staging regex ends with `\b` after an escaped
period, which in ERE is not a word boundary at end-of-string — so the dot form of staging the
current directory escapes that check while the flag form matches.

**Status:** pre-existing; `git-add-guard.sh` blocks the pattern independently, so protection
holds. **Detection:** exercise a hook regex with _every literal variant_, not just one.

## `strip_embedded_content` Cannot Strip a Multi-Line `-m` Body (PRE-EXISTING)

**What happened:** phase 2 of `strip_embedded_content` in `hooks/_common.sh` is a line-oriented
pattern for `-m` followed by a quoted run, so a quoted body spanning newlines never matches and
survives into the "stripped" string. `git-add-guard.sh` Pattern 1 then matches the staging verb
followed by an unanchored `.*` spanning the whole line — wrongly blocking a legitimate
"stage one file && commit with a multi-line message that merely _mentions_ those flags". It is
not limited to git: it blocked a plain `loom memory note` whose _text_ quoted the all-files flags.

**Workaround:** split staging and committing into separate Bash calls, and keep the all-files
flag spellings out of message and note text.
**Real fix (LANDED 2026-08-11):** see the next section — the raw-string scan was replaced by a
token scan, so the workaround is no longer needed for `git-add-guard.sh`.

## Regexing a Raw Command String Matches Prose Inside Quoted Arguments (2026-08-11)

**What happened:** `git-add-guard.sh` stripped heredoc and `-m` bodies and then ran its three
patterns over the **raw command string**. Quoted argument bodies were never stripped, so a
command that merely _mentioned_ the forbidden forms inside a quoted argument was blocked with no
git invocation anywhere in it. Confirmed: `echo 'Never run git add -A or git add . because it
stages .work'` was blocked. This is the section above one level out — same class, wider blast
radius: `echo`, Write-via-shell, subagent prompt forwarding, and any doc or prompt text about
staging hygiene all tripped it. It blocked real work in this repo, including a probe command
written to _test_ the guard.

**Why:** a regex over a command string cannot tell an argument's **value** from an argument's
**mention**. Stripping more kinds of body is a treadmill; the string is the wrong unit.

**The tempting wrong fix:** blank the interiors of quoted spans before matching. It fixes the
false positive and opens a real hole — bash quoting does not change an argument's value, so
`git add '-A'` and `git add ".work"` start passing. Verified against the pre-fix hook: those two
were **already** allowed, along with `echo $(git add -A)` (Pattern 1's boundary class omitted
`)`). Three silent false negatives sat behind the false positive nobody could miss.

**Prevention:** scan **tokens, not text**. Tokenize with quote/escape state, emit a sentinel at
each command boundary, then check tokens that are genuinely arguments of the invocation you care
about. A quoted mention is one token belonging to `echo`; a quoted real flag keeps its value and
is still caught. When a guard has a loud false positive, look for the quiet false negatives of
the same root cause before fixing only the loud one.

**Fix:** `loom_tokenize_command` in `hooks/_common.sh` (permissive: tolerates any shell text,
returns non-zero only on an unterminated quote) plus `scan_git_add_tokens` in
`hooks/git-add-guard.sh`; the old regex block is retained solely as the unterminated-quote
fallback so protection never drops below its previous level. Cases in
`hooks/tests/git-add-guard-quoting.sh`.

**Swept 2026-08-26 (was "deliberately NOT swept").** The deferral above cost real work: the three
deferred hooks kept blocking codex briefs for months, and the deferral note is what identified
them when it finally bit. `commit-filter.sh`, `prefer-modern-tools.sh` and `worktree-isolation.sh`
now scan tokens too, sharing seven `loom_tokens_*` helpers in `_common.sh` rather than each
re-implementing the walk. `strip_embedded_content` is still unchanged and still runs FIRST — it
strips heredoc bodies, which would otherwise tokenize as real command words.
`subagent-verify-guard.sh` remains on raw strings and still carries this bug class.

**The conversion itself is the dangerous part — see the section below.**

## Sourcing a Bash-Targeted Hook From an Interactive zsh Gives Bogus Results (2026-08-26)

**What happened:** ad-hoc probes of `loom_tokenize_command` produced results that contradicted
each other run to run — the same input appeared to splice `sh -c` payloads in one invocation and
not the next. Nothing was flaky. The probes run via `bash <file>` on a saved script were correct;
the ones typed inline were evaluated by the session's **interactive zsh**, where arrays are
1-based, so every index computed by the walker was off by one and the token dumps were
meaningless. Nearly led to a fabricated "non-deterministic tokenizer" bug report.

**Why:** the default shell here is zsh; `hooks/*.sh` are all `#!/usr/bin/env bash` and are only
ever executed by bash in production. Sourcing one into zsh runs bash-targeted array code under
different semantics with no error.

**Prevention:** never verify a hook by sourcing it inline. Put the probe in a file with a bash
shebang and run it with `bash <file>`, and have the probe print `$BASH_VERSION` if there is any
doubt. When two runs of "the same" check disagree, suspect the interpreter before the code.

## Converting a Raw-String Matcher to Token Scanning Silently Narrows It (2026-08-26)

**What happened:** converting `commit-filter.sh`, `worktree-isolation.sh` and
`prefer-modern-tools.sh` from regex-on-raw-string to argv-token scanning removed the documented
false positives AND opened **seven bypasses**, every one of which the old regex had blocked. The
full suite was green — 3090 Rust tests, 39 hook tests, clippy, fmt — and an adversarial review
found six of them; a self-directed pass afterwards found the seventh. Confirmed bypasses:
`bash -c 'git commit'`, `if git commit; then`, `timeout 60 git commit`, `git $'commit'`,
`exec git commit`, a `loom stage complete` split across two segments, and a triple-nested
`bash -c`.

**Why:** a raw regex matches the words **anywhere**; token scanning matches them only at an argv
position the walker actually reaches. Every construct the walker does not model — a shell's `-c`
payload, a keyword or builtin occupying argv[0], an arg-taking wrapper flag, an alternate quoting
spelling — becomes a hole. The tests measured the false positives being removed; nothing measured
the coverage being given up.

**Prevention:** when replacing a matcher, the acceptance criterion is not "the new tests pass" but
**"every input the old matcher blocked is still blocked."** Keep the old pattern runnable and diff
the two over an adversarial corpus. Concretely: `printf '%s' "$cmd" | grep -qiE '<old pattern>'`
next to the new predicate, and a case list of command-word obfuscations (`command`/`exec`,
keywords, wrappers, `VAR=`, absolute and `./` paths, `$'...'`, separators, `$( )`, pipes,
here-strings, process substitution). A green suite is not evidence here — it never was.

**A bounded walker must fail toward the block.** The `sh -c` splice recursion stopped at depth 2
and still returned success, so a triple-nested payload was trusted as fully walked. Exhausting the
budget now returns failure, which routes callers to the stricter raw-regex fallback. Any fixed
bound has a next level; what matters is which way it fails.

**Fix:** `hooks/_common.sh` — `sh -c` payloads spliced (real shells only: `grep -c`, `sort -c`,
`wc -c` must not match), transparent keywords and command-prefix builtins, arg-taking wrapper
flags, ANSI-C quoting, a substitution-state stack so `$(` inside double quotes no longer aborts
the parse, and the depth-budget fail-safe. Cases in `hooks/tests/common-token-helpers.sh`.

**Two residuals, both pre-existing (the old regex allowed them too, so not regressions):**
`${GITBIN} commit` (parameter expansion at a command position) and `git $'\x63ommit'` (ANSI-C hex
escape). Verify any future "bypass" against the OLD pattern before calling it a regression.

## Bash: `'\\'` in a `case` Pattern Never Matches a Lone Backslash (2026-08-11)

**What happened:** `parse_shell_words` in `hooks/codex-forward-guard.sh` used `'\\')` as the
`case` arm meant to catch a backslash, in three places. In a `case` pattern a quoted string has
its metacharacters disabled, so `'\\'` is the literal **two-character** string `\\` and can never
match the single backslash the parser iterates over. Both the `escape` and `double_escape` states
were therefore unreachable dead code.

**Why it mattered:** a backslash fell through to `*)`, was appended as a literal, and left the
parser in `plain`. The standard `'\''` idiom for embedding an apostrophe then knocked the state
machine out of phase, so the rest of the argument was scanned in `plain` where spaces split words
and `( ) $ * { }` hard-reject. Net effect: **any prompt containing an ASCII apostrophe was
unforwardable**, which silently disabled the whole codex implementation lane. The symptom looked
like a backtick or quoting problem in the prompt, not a dead `case` arm.

**Prevention:** to match a lone backslash in a `case` pattern use `'\'` or unquoted `\\` — both
verified. More generally: an unreachable `case` arm in a state machine is invisible, because the
fall-through arm produces plausible output. When adding a state, assert the state is actually
entered (parse a known input and check the **parsed words**, not just the exit code) — a
pass/fail exit code cannot distinguish "handled correctly" from "never reached".

**Fix:** three patterns in `hooks/codex-forward-guard.sh`; round-trip cases (apostrophe idiom,
`\$`, `\"`, `\\`, trailing lone backslash) in `hooks/tests/codex-forward-guard-quoting.sh`, which
asserts the parsed word content, not merely the hook's exit code.

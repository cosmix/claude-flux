#!/usr/bin/env python3
"""skill-trigger - UserPromptSubmit hook for keyword-based skill suggestions.

Reads user prompts via stdin JSON and outputs skill suggestions to stdout.
Suggestions are injected into Claude's context to encourage skill usage.

Input: JSON from stdin with structure:
  {"session_id": "...", "prompt": "Help me implement JWT authentication"}

Output: JSON with hookSpecificOutput.additionalContext for context injection.
  Plain text stdout from UserPromptSubmit hooks is unreliable (see claude-code#13912).

  A skill installed in ~/.claude/skills keeps its slash-command form:
    - /loom-plan-writer -- <desc> (matched: kw)
  A catalogued skill has no slash command, so the line names the loader instead:
    - loom-rust -- <desc> (matched: kw) -- load with Skill(skill="loom-skills", args="loom-rust")
  When two or more catalogued skills qualify, a final line offers one
  combined load call instead of one per skill:
    All catalogued matches at once: Skill(skill="loom-skills", args="loom-rust loom-react")

Dependencies:
  ~/.claude/hooks/loom/skill-keywords.json (built by `loom skill-index`)
  ~/.claude/loom-skill-catalog/<name>/SKILL.md for descriptions of catalogued skills
"""

import json
import os
import re
import sys

INDEX_FILE = os.path.expanduser("~/.claude/hooks/loom/skill-keywords.json")
SKILLS_DIR = os.path.expanduser("~/.claude/skills")
CATALOG_DIR = os.path.expanduser("~/.claude/loom-skill-catalog")
DEBUG_LOG = os.path.expanduser("~/.claude/hooks/loom/skill-trigger.log")
# Flood ceiling only - every qualifying skill (score >= MIN_SCORE) is listed;
# this just guards against a pasted document matching dozens of skills at once.
MAX_SUGGESTIONS = 8
MIN_SCORE = 2  # Minimum weighted score to suggest a skill
DEBUG = os.environ.get("LOOM_SKILL_DEBUG", "") == "1"

# Words too generic to be meaningful skill triggers on their own.
# Multi-word keywords containing these are still allowed (e.g. "access control").
STOPWORDS = frozenset({
    # Common programming verbs
    "add", "build", "change", "check", "close", "copy", "create", "debug",
    "delete", "deploy", "find", "fix", "get", "help", "install", "list",
    "make", "move", "open", "pull", "push", "read", "remove", "run", "send",
    "set", "show", "start", "stop", "test", "update", "use", "write",
    # Common nouns
    "app", "bug", "class", "code", "config", "data", "error", "file",
    "function", "issue", "log", "method", "new", "old", "output", "plan",
    "project", "script", "setup", "tool", "type", "value",
    # Project-specific words that appear in nearly every prompt
    "claude", "loom",
})


def _detect_languages(cwd):
    """Return a set of language/framework tokens derived from manifest files
    in `cwd`. Tokens are fed into scoring at weight 1 (no name-match boost)
    so they reinforce user-typed signals without triggering skills on their
    own — a Cargo.toml alone won't suggest loom-rust on an unrelated prompt,
    but "help me with lifetimes" in a Rust repo will.
    """
    detected = set()

    def has(*names):
        return any(os.path.isfile(os.path.join(cwd, n)) for n in names)

    def has_dir(*names):
        return any(os.path.isdir(os.path.join(cwd, n)) for n in names)

    # Languages (backend)
    if has("Cargo.toml"):
        detected.add("rust")
    if has("go.mod", "go.sum"):
        detected.add("golang")
    if has("pyproject.toml", "setup.py", "requirements.txt", "Pipfile",
           "poetry.lock"):
        detected.add("python")

    # JS/TS ecosystem
    if has("tsconfig.json", "deno.json", "bun.lockb", "bun.lock"):
        detected.add("typescript")
    # Next.js / Remix config files imply React even without reading deps
    if has("next.config.js", "next.config.ts", "next.config.mjs",
           "next.config.cjs", "remix.config.js"):
        detected.add("react")

    # package.json: cheapest reliable framework signal is dep inspection
    pkg_path = os.path.join(cwd, "package.json")
    if os.path.isfile(pkg_path):
        try:
            with open(pkg_path) as f:
                pkg = json.load(f)
            deps = {}
            for section in ("dependencies", "devDependencies",
                            "peerDependencies"):
                deps.update(pkg.get(section) or {})
            if any(d in deps for d in
                   ("react", "react-dom", "next", "remix",
                    "@remix-run/react", "@remix-run/node")):
                detected.add("react")
            if "typescript" in deps:
                detected.add("typescript")
        except (IOError, json.JSONDecodeError):
            pass

    # Containers & build
    if has("Dockerfile", "docker-compose.yml", "docker-compose.yaml",
           "compose.yaml", "compose.yml"):
        detected.add("docker")

    # Kubernetes (Helm / Kustomize / Skaffold)
    if has("kustomization.yaml", "kustomization.yml"):
        detected.update(("kustomize", "kubernetes"))
    if has("Chart.yaml", "helmfile.yaml", "helmfile.yml", "skaffold.yaml"):
        detected.add("kubernetes")

    # Terraform / IaC
    if has("main.tf", "terraform.tf", ".terraform.lock.hcl", "versions.tf"):
        detected.add("terraform")

    # CI/CD pipelines — multi-word token matches loom-ci-cd index entry
    if has_dir(".github/workflows") or has(
        ".gitlab-ci.yml", ".circleci/config.yml", "azure-pipelines.yml",
        "Jenkinsfile",
    ):
        detected.add("ci/cd")

    # GitOps tooling
    if has_dir("argocd", ".argocd"):
        detected.add("argocd")
    if has_dir("flux", ".flux") or has("flux-system.yaml"):
        detected.add("fluxcd")

    # Observability
    if has("prometheus.yml", "prometheus.yaml"):
        detected.add("prometheus")
    if has_dir("grafana") or has("grafana.ini"):
        detected.add("grafana")

    return detected


def _is_name_match(keyword, skill_name):
    """True when keyword strongly identifies the skill by name.

    Every shipped skill is named `loom-<topic>`, so we strip the `loom-`
    prefix before comparing. Exact match ("rust" == "rust") or prefix
    match with min length 4 ("refactor" -> "refactoring") to avoid
    short false matches.
    """
    effective = skill_name[5:] if skill_name.startswith("loom-") else skill_name
    if keyword == effective:
        return True
    if len(keyword) >= 4 and effective.startswith(keyword):
        return True
    return False


def _debug(msg):
    """Write debug message to log file if LOOM_SKILL_DEBUG=1."""
    if not DEBUG:
        return
    try:
        import datetime

        ts = datetime.datetime.now().strftime("%H:%M:%S")
        with open(DEBUG_LOG, "a") as f:
            f.write(f"[{ts}] {msg}\n")
    except IOError:
        pass


def main():
    if sys.stdin.isatty():
        _debug("SKIP: stdin is tty")
        return

    try:
        data = json.load(sys.stdin)
    except (json.JSONDecodeError, ValueError):
        _debug("SKIP: invalid JSON on stdin")
        return

    prompt = data.get("prompt", "")
    _debug(f"FIRED: prompt={prompt[:80]!r}")
    if not prompt or not os.path.isfile(INDEX_FILE):
        _debug(f"SKIP: empty prompt or no index (index exists: {os.path.isfile(INDEX_FILE)})")
        return

    try:
        with open(INDEX_FILE) as f:
            index = json.load(f)
    except (json.JSONDecodeError, IOError):
        return

    # Tokenize: lowercase, keep special chars like / - . within words
    words = re.findall(r"[a-z0-9]+(?:[/._-][a-z0-9]+)*", prompt.lower())
    # Filter stopwords from single-word tokens, but allow a stopword through
    # if the indexer chose to keep it (e.g. "test" is a stopword but names
    # the loom-testing skill, so the indexer exempts it). This lets primary
    # skill verbs like "test" and "debug" actually trigger their skills.
    tokens = set(
        w for w in words if len(w) > 1 and (w not in STOPWORDS or w in index)
    )

    # Generate bigrams and trigrams for multi-word keyword matching
    # e.g. "event sourcing", "api key", "access control"
    # These are NOT stopword-filtered since multi-word phrases are specific enough
    for i in range(len(words) - 1):
        tokens.add(f"{words[i]} {words[i + 1]}")
    for i in range(len(words) - 2):
        tokens.add(f"{words[i]} {words[i + 1]} {words[i + 2]}")

    # Fold common English plurals to their singular form when the original
    # token isn't indexed but the stemmed form is. Safe because we only
    # produce a stem if it hits the index. Stems must be >= 3 chars to
    # avoid short false positives like "goes" -> "go" (loom-golang) or
    # "does" -> "do". Applies to the last word of bigrams/trigrams too so
    # "feature flags" matches the singular "feature flag" entry.
    stemmed = set()
    for t in tokens:
        if t in index:
            continue
        parts = t.split(" ")
        last = parts[-1]
        stems = []
        if last.endswith("s") and len(last) > 3:
            stems.append(last[:-1])
        if last.endswith("es") and len(last) > 4:
            stems.append(last[:-2])
        for stem in stems:
            cand = " ".join(parts[:-1] + [stem]) if len(parts) > 1 else stem
            if cand in index:
                stemmed.add(cand)
                break
    tokens.update(stemmed)

    # Score skills by keyword matches
    # Multi-word matches (containing space) count double since they're more specific
    # Direct skill-name matches get boosted weight (high-confidence signal)
    scores = {}
    matched = {}
    for token in tokens:
        if token in index:
            base_weight = 2 if " " in token else 1
            for skill in index[token]:
                w = base_weight
                # Boost: keyword directly identifies the skill by name
                # e.g., "rust" -> "rust", "refactor" -> "refactoring"
                if base_weight == 1 and _is_name_match(token, skill):
                    w = 2
                scores[skill] = scores.get(skill, 0) + w
                matched.setdefault(skill, []).append(token)

    # Ambient repo context: inject detected languages/frameworks at weight 1
    # (no name-match boost) so they reinforce rather than solo-trigger. Skip
    # anything the user already typed to avoid double-counting the same signal.
    cwd = data.get("cwd") or os.getcwd()
    for lang in _detect_languages(cwd):
        if lang in tokens or lang not in index:
            continue
        for skill in index[lang]:
            scores[skill] = scores.get(skill, 0) + 1
            matched.setdefault(skill, []).append(f"repo:{lang}")

    if not scores:
        return

    # Filter skills below minimum score threshold
    qualified = {s: sc for s, sc in scores.items() if sc >= MIN_SCORE}
    if not qualified:
        return

    # "loom-skills" is the catalog loader itself; suggesting it is redundant
    # once any other qualified skill already names the loader in its own
    # line. Keep it when it's the only qualified skill so a prompt about
    # "skills" in general still points somewhere.
    if len(qualified) > 1 and "loom-skills" in qualified:
        del qualified["loom-skills"]

    # Sort by score desc, then by number of distinct matched keywords desc,
    # then by name. The name tiebreaker matters because Python's set
    # iteration order is seeded per process (PYTHONHASHSEED) — without it,
    # equal-score ties would order differently across runs.
    top = sorted(
        qualified.items(),
        key=lambda kv: (-kv[1], -len(matched[kv[0]]), kv[0]),
    )[:MAX_SUGGESTIONS]

    context = _render(top, matched)
    if context:
        _debug(f"SUGGEST: {context}")
        # Use JSON additionalContext format — plain text stdout is unreliable
        # for UserPromptSubmit hooks (see claude-code#13912)
        print(json.dumps({
            "hookSpecificOutput": {
                "hookEventName": "UserPromptSubmit",
                "additionalContext": context,
            }
        }))
    elif DEBUG:
        _debug(f"NO MATCH: scores={scores}")


def _render(top, matched):
    """Build the additionalContext string for the ranked (skill, score)
    pairs in `top`. Returns None when there is nothing to render.
    """
    lines = []
    catalogued_names = []
    for skill, _score in top:
        # Sorted for deterministic display — `matched[skill]` was built by
        # iterating a set (tokens), whose order varies with PYTHONHASHSEED.
        kws = ", ".join(sorted(matched[skill])[:4])
        path, catalogued = _locate_skill_md(skill)
        desc = _parse_description(path) if path else ""
        if catalogued:
            # Not indexed by Claude Code, so there's no /{skill} slash
            # command to name — point at the loader invocation instead.
            catalogued_names.append(skill)
            loader = f'Skill(skill="loom-skills", args="{skill}")'
            if desc:
                lines.append(f"  - {skill} -- {desc} (matched: {kws}) -- load with {loader}")
            else:
                lines.append(f"  - {skill} (matched: {kws}) -- load with {loader}")
        elif desc:
            lines.append(f"  - /{skill} -- {desc} (matched: {kws})")
        else:
            lines.append(f"  - /{skill} (matched: {kws})")

    if not lines:
        return None

    if len(catalogued_names) >= 2:
        combined = " ".join(catalogued_names)
        lines.append(
            f'  All catalogued matches at once: Skill(skill="loom-skills", args="{combined}")'
        )

    context = (
        "SKILL MATCH: These skills are relevant to this task. Load EVERY one "
        "that applies before implementing, one at a time with the calls below "
        "or all catalogued ones in a single call:\n"
    )
    context += "\n".join(lines)
    return context


def _locate_skill_md(skill_name):
    """Find a skill's SKILL.md, checking the installed directory before the
    catalog. Returns (path, catalogued): catalogued is True when the skill
    was found under CATALOG_DIR rather than SKILLS_DIR. Returns (None,
    False) when neither directory has it (a stale index entry).
    """
    installed = os.path.join(SKILLS_DIR, skill_name, "SKILL.md")
    if os.path.isfile(installed):
        return installed, False
    catalogued = os.path.join(CATALOG_DIR, skill_name, "SKILL.md")
    if os.path.isfile(catalogued):
        return catalogued, True
    return None, False


def _parse_description(path):
    """Extract short description from a SKILL.md file's frontmatter."""
    try:
        with open(path) as f:
            text = f.read(2000)
    except IOError:
        return ""
    m = re.search(r"^---\s*\n(.*?)\n---", text, re.DOTALL)
    if not m:
        return ""
    fm = m.group(1)
    # Multiline description (|)
    m = re.search(r"^description:\s*\|\s*\n\s+(.+)", fm, re.MULTILINE)
    if m:
        d = m.group(1).strip()
    else:
        # Inline description
        m = re.search(r"^description:\s*(.+)", fm, re.MULTILINE)
        if not m:
            return ""
        d = m.group(1).strip()
    # Truncate at first natural break point
    for marker in [". Trigger", ". Use when", ". Covers", ". Keywords", ". Primary"]:
        idx = d.find(marker)
        if 0 < idx < 80:
            d = d[: idx + 1]
            break
    return d[:80]


if __name__ == "__main__":
    main()

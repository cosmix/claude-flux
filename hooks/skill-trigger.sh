#!/usr/bin/env python3
"""Suggest skills from prompt keywords and Loom's shared project discovery.

Claude uses Skill-tool invocations; --codex emits native SKILL.md read paths.
Project discovery is bounded and delegated to loom hook project-types.
The hook is advisory: unavailable discovery never blocks a user prompt.

A detected repository type is a tie-breaker, never a qualifier: it adds one
point to a skill's score, which only clears MIN_SCORE alongside a prompt
keyword hit of its own. A repo-type skill never qualifies from detection
alone, so a prompt with no matching keywords gets no suggestions just
because a technology exists somewhere in the tree.
"""

import json
import os
import re
import subprocess
import sys

CODEX = "--codex" in sys.argv[1:]
MAX_SUGGESTIONS = 5
MIN_SCORE = 2
DEBUG = os.environ.get("LOOM_SKILL_DEBUG", "") == "1"
STOPWORDS = frozenset({
    "add", "build", "change", "check", "close", "copy", "create", "debug",
    "delete", "deploy", "find", "fix", "get", "help", "install", "list",
    "make", "move", "open", "pull", "push", "read", "remove", "run", "send",
    "set", "show", "start", "stop", "test", "update", "use", "write",
    "app", "bug", "class", "code", "config", "data", "error", "file",
    "function", "issue", "log", "method", "new", "old", "output", "plan",
    "project", "script", "setup", "tool", "type", "value", "claude", "loom",
})


def _agent_root():
    # Respect explicit installation roots, including --codex-dir installs.
    hook_dir = os.path.dirname(os.path.abspath(__file__))
    if os.path.basename(hook_dir) == "loom" and os.path.basename(os.path.dirname(hook_dir)) == "hooks":
        return os.path.dirname(os.path.dirname(hook_dir))
    if CODEX:
        return os.environ.get("CODEX_HOME") or os.path.expanduser("~/.codex")
    return os.path.expanduser("~/.claude")


AGENT_ROOT = _agent_root()
INDEX_FILE = os.path.join(AGENT_ROOT, "hooks/loom/skill-keywords.json")
SKILLS_DIR = os.path.join(AGENT_ROOT, "skills")
CATALOG_DIR = os.path.join(AGENT_ROOT, "loom-skill-catalog")
DEBUG_LOG = os.path.join(AGENT_ROOT, "hooks/loom/skill-trigger.log")


def _debug(msg):
    if DEBUG:
        try:
            with open(DEBUG_LOG, "a") as log:
                log.write(msg + "\n")
        except OSError:
            pass


def _is_name_match(keyword, skill_name):
    effective = skill_name[5:] if skill_name.startswith("loom-") else skill_name
    return keyword == effective or (len(keyword) >= 4 and effective.startswith(keyword))


def _load_index():
    try:
        with open(INDEX_FILE) as source:
            index = json.load(source)
        return {key: names for key, names in index.items()
                if isinstance(key, str) and isinstance(names, list)
                and all(isinstance(name, str) for name in names)}
    except (OSError, ValueError, AttributeError):
        return {}


def _tokens(prompt, index):
    words = re.findall(r"[a-z0-9]+(?:[/._-][a-z0-9]+)*", prompt.lower())
    tokens = {w for w in words if len(w) > 1 and (w not in STOPWORDS or w in index)}
    for size in (2, 3):
        tokens.update(" ".join(words[i:i + size]) for i in range(len(words) - size + 1))
    stemmed = set()
    for token in tokens:
        if token in index:
            continue
        parts = token.split(" ")
        last = parts[-1]
        stems = []
        if last.endswith("s") and len(last) > 3:
            stems.append(last[:-1])
        if last.endswith("es") and len(last) > 4:
            stems.append(last[:-2])
        for stem in stems:
            candidate = " ".join(parts[:-1] + [stem])
            if candidate in index:
                stemmed.add(candidate)
                break
    return tokens | stemmed


def _score_keywords(tokens, index):
    scores, matched = {}, {}
    for token in tokens:
        for skill in index.get(token, []):
            weight = 2 if " " in token or _is_name_match(token, skill) else 1
            scores[skill] = scores.get(skill, 0) + weight
            matched.setdefault(skill, []).append(token)
    return scores, matched


def _project_types(cwd, prompt):
    try:
        result = subprocess.run(
            [os.environ.get("LOOM_BIN") or "loom", "hook", "project-types"],
            input=json.dumps({"cwd": cwd, "prompt": prompt}),
            capture_output=True, text=True, timeout=3, check=True,
        )
        profile = json.loads(result.stdout)
        types = profile.get("types", [])
        if isinstance(types, list):
            return profile, [item for item in types if isinstance(item, dict)
                             and isinstance(item.get("kind"), str)
                             and isinstance(item.get("path"), str)]
    except (OSError, ValueError, AttributeError, subprocess.SubprocessError) as error:
        _debug(f"Project discovery unavailable: {error}")
    return {}, []


def _skill_roots(cwd, root):
    roots = []
    if CODEX:
        current = os.path.realpath(cwd)
        boundary = os.path.realpath(root or cwd)
        while current == boundary or current.startswith(boundary + os.sep):
            roots.append(os.path.join(current, ".agents/skills"))
            if current == boundary:
                break
            current = os.path.dirname(current)
        roots.append(os.path.expanduser("~/.agents/skills"))
    roots.extend((SKILLS_DIR, CATALOG_DIR))
    return roots


def _locate_skill_md(skill_name, roots):
    if not re.fullmatch(r"[a-zA-Z0-9_-]+", skill_name):
        return None, False
    for root in roots:
        path = os.path.join(root, skill_name, "SKILL.md")
        if os.path.isfile(path):
            return path, root == CATALOG_DIR
    return None, False


def _add_project_matches(types, roots, scores, matched):
    for item in types:
        kind = item["kind"]
        # Detection identifies a skill directly. "react" also indexes TypeScript;
        # expanding detected types through that index caused asymmetric scoring.
        # A detected type is only a tie-breaker: it adds one point, never enough
        # on its own to clear MIN_SCORE, so a repo-type skill still needs a
        # prompt keyword hit of its own to qualify.
        for skill in ("loom-" + kind, kind):
            if _locate_skill_md(skill, roots)[0]:
                scores[skill] = scores.get(skill, 0) + 1
                path = json.dumps(item["path"] or ".")[1:-1]
                marker = f"repo:{kind} ({path})"
                if marker not in matched.setdefault(skill, []):
                    matched[skill].append(marker)
                break


def _rank(scores, matched):
    qualified = {name: score for name, score in scores.items() if score >= MIN_SCORE}
    if len(qualified) > 1:
        qualified.pop("loom-skills", None)
    return sorted(qualified.items(), key=lambda item: (
        -item[1], -len(matched[item[0]]), item[0],
    ))[:MAX_SUGGESTIONS]


def _repo_kind(skill, matched):
    for entry in matched.get(skill, []):
        if entry.startswith("repo:"):
            return entry[len("repo:"):].split(" (", 1)[0]
    return None


def _render_one(skill, matched, roots, keyword_scores):
    path, catalogued = _locate_skill_md(skill, roots)
    if not path:
        return None, False
    keywords = ", ".join(sorted(matched[skill])[:4])
    desc = _parse_description(path)
    label = f"{skill} -- {desc}" if desc else skill
    if CODEX:
        if keyword_scores.get(skill, 0) >= MIN_SCORE:
            return f"  - {label} (matched: {keywords}) -- read {json.dumps(path)} in full", False
        kind = _repo_kind(skill, matched) or "this"
        return (f"  - {label} (matched: {keywords}) -- read {json.dumps(path)} "
                f"if the task touches {kind}"), False
    if catalogued:
        loader = f'Skill(skill="loom-skills", args="{skill}")'
        return f"  - {label} (matched: {keywords}) -- load with {loader}", True
    return f"  - /{label} (matched: {keywords})", False


def _render(top, matched, roots, keyword_scores):
    lines, catalogued = [], []
    for skill, _score in top:
        line, is_catalogued = _render_one(skill, matched, roots, keyword_scores)
        if line:
            lines.append(line)
        if is_catalogued:
            catalogued.append(skill)
    if not lines:
        return None
    if len(catalogued) >= 2:
        combined = " ".join(catalogued)
        lines.append(f'  All catalogued matches at once: Skill(skill="loom-skills", args="{combined}")')
    return ("SKILL MATCH: skills matching this request (keyword hits shown; "
            "repo: markers are context, not a reason to load).\n"
            + "\n".join(lines))


def _parse_description(path):
    try:
        with open(path) as source:
            text = source.read(2000)
    except OSError:
        return ""
    frontmatter = re.search(r"^---\s*\n(.*?)\n---", text, re.DOTALL)
    if not frontmatter:
        return ""
    match = re.search(r"^description:\s*\|\s*\n\s+(.+)", frontmatter[1], re.MULTILINE)
    if not match:
        match = re.search(r"^description:\s*(.+)", frontmatter[1], re.MULTILINE)
    if not match:
        return ""
    description = match[1].strip()
    for marker in [". Trigger", ". Use when", ". Covers", ". Keywords", ". Primary"]:
        index = description.find(marker)
        if 0 < index < 80:
            description = description[:index + 1]
            break
    return description[:80]


def main():
    if sys.stdin.isatty():
        return
    try:
        data = json.loads(sys.stdin.read(1024 * 1024 + 1))
    except ValueError:
        return
    if not isinstance(data, dict) or not isinstance(data.get("prompt"), str) or not data["prompt"]:
        return
    prompt = data["prompt"]
    cwd = data.get("cwd") or os.getcwd()
    if not isinstance(cwd, str):
        return
    index = _load_index()
    scores, matched = _score_keywords(_tokens(prompt, index), index)
    profile, types = _project_types(cwd, prompt)
    roots = _skill_roots(cwd, profile.get("root"))
    scores = {name: score for name, score in scores.items() if _locate_skill_md(name, roots)[0]}
    keyword_scores = dict(scores)
    _add_project_matches(types, roots, scores, matched)
    context = _render(_rank(scores, matched), matched, roots, keyword_scores)
    if context:
        print(json.dumps({"hookSpecificOutput": {
            "hookEventName": "UserPromptSubmit", "additionalContext": context,
        }}))


if __name__ == "__main__":
    main()

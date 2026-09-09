# Automatic skill routing

Loom discovers project types for both Claude Code and Codex. Prompt hooks and
stage signals share the detector in `loom/src/skills/project.rs`.

The detector finds nested packages and associates each language, framework, or
infrastructure tool with its directory. From a checkout root, suggestions can
cover every package. A prompt naming a file or directory selects its package;
otherwise the current directory determines scope. Stage signals use their
assigned files and globs. Source extensions constrain language-specific
assignments, so `**/*.rs` does not select React from a sibling web package.

Detection covers Rust, TypeScript, Python, Go, React, Docker, Kubernetes,
Kustomize, Terraform, CI/CD, Argo CD, Flux, Prometheus, and Grafana. TypeScript
requires TypeScript evidence: an arbitrary `package.json` or Bun lockfile does
not establish it. Dependency and build directories, symlinked directories, and
nested checkouts are excluded. Traversal stops at eight levels or 20,000 entries;
the internal `loom hook project-types` response reports truncation.

Detected types resolve directly to their corresponding installed skills.
They qualify without matching words in the user prompt and without a generated
keyword index. Keyword matching remains available for task-specific skills,
with deterministic ranking and an eight-suggestion cap. A stage's existing
keyword match is promoted when detection identifies the same skill, preserving
the directive to load it before editing.

Claude receives its normal Skill-tool or catalog-loader invocation. Codex
receives concrete `SKILL.md` paths to read in full, using its own installation
and preferring matching native `.agents/skills` entries. Both clients' read
guards permit full reads of their skill files, including large catalog entries.
Hooks recommend loading; the agent selects the suggestions that apply.

`loom skill-index` refreshes separate Claude and Codex keyword indexes.
The release installer and `loom update` automatically install both clients'
hooks and regenerate both indexes through the existing `loom install-assets`
command. No separate asset-install step is needed after installing a release.
Codex may require review of changed hooks through `/hooks`.

To inspect project facts without running an agent:

```bash
printf '%s\n' '{"cwd":"/absolute/project/path","prompt":"edit web/src/App.tsx"}' \
  | loom hook project-types
```

Discovery performs no model or network calls and is recomputed for each prompt,
so manifest edits need no cache refresh. The hook limits the discovery subprocess
to three seconds; failures leave ordinary keyword suggestions available.

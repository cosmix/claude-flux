---
sources:
- loom/src/context/pack.rs
- loom/src/context/retrieve.rs
verified: 054528e508d51ede343e254590cdb73ae00f7df6
---
# Context Retrieval

> The retrieval subsystem: two graphs, two lanes, query-side gating, two-tier fusion, and the persistent BM25 index.

## What This Subsystem Is

`loom/src/context/` answers exactly one question — "which curated prose is worth
spending N tokens on for this query?" — and answers it identically every time.
No embedding model, no network call, no randomness: a `ContextPack` is a pure
function of the bytes on disk and the query string (the `context` module doc).

Read `context/mod.rs` first for the module map. Its pipeline prose now describes
both channels and two-tier fusion, so it is no longer the stale docstring an
earlier version of this section warned about; its diagram still omits prose
indexing, the lexical index, lifecycle filtering and source-lane expansion, which
this file covers.

## Two Graphs, Two Lanes, Both Wired

There are two distinct graphs.

| Graph | Built by | Ranked by | Consumed by |
| --- | --- | --- | --- |
| Knowledge-chunk catalog (curated prose under `doc/loom/knowledge/`, plus indexed project prose — see below) | `fs::knowledge::chunker`/`catalog::prose` → `context::ingest` | `context::rank` | `loom knowledge context`, `loom knowledge eval`, the Knowledge Brief, the prompt hook |
| Source graph (tree-sitter nodes/edges over the repo) | `context::extract` → `context::refresh::source_graph` | `context::rank_source` | the same, plus `loom map` via `context::graph_store` |

`retrieve/channels.rs::rank_channels_cached` dispatches per channel:
`Channel::Knowledge` to the knowledge ranker (then `apply_lifecycle_policy`),
`Channel::Source` to `rank_source_channel_cached` when a resolved graph is
available and to an empty result when it is not. Each channel gets its own
persistent lexical cache when a cache root exists. The two lists then meet in
`fuse` → `pack`, so a pack can mix curated prose, indexed prose and symbol nodes.

**`rank_source` is not `rank` with a different corpus, but the two share the
exact-rung machinery.** `context/rank_source.rs`:

- Whole-**file** nodes are dropped first — no signature, no scope, nothing to
  score.
- The exact-match rungs (`ExplicitId` / `ExactPath` / `ExactSymbol`) match
  against the RAW query text via `lexical::contains_whole_term`, not against
  tokens: `tokenize` would shred CamelCase symbols and slashed paths
  (`rank_source/paths.rs`).
- Both rankers gate every rung through the same `lexical::ExactGate` and
  accumulate through the same `rank::RungScore` — see **Exact-Rung Gating**
  below. `prepare_lexical` / `score_bm25` live in `rank::corpus` so both rankers
  score identically; only their document construction differs (`node_document`
  builds a source document from `node.scope` at `WEIGHT_SYMBOLS` plus
  `node.signature` at `WEIGHT_BODY`).
- A source node in a file a dependency stage owns earns `StageDependency`
  (`BOOST_STAGE_DEPENDENCY = 30.0`), matched on exact normalized paths — a
  prefix match would hand the boost to an entire tree
  (`paths::names_dependency_path`).
- A node whose path or scope follows a test convention has its FINAL score
  (rungs plus lexical) multiplied by `config.test_path_factor` (default
  `0.4`) — ordering pressure so implementation outranks the tests that
  exercise it, never exclusion (`paths::apply_test_path_factor`).
- Candidates, graph neighbours included, are cut to `MAX_SOURCE_CANDIDATES = 60`;
  ties break on `(path, line_start)` so the pack is deterministic (`rank_order`).
- **Coverage guard:** a node whose file was not fully extracted loses ALL THREE
  high-confidence rungs together, not selectively (`withhold_partial_coverage`).
  `Confidence::from_reasons` promotes to `High` on any one of the three, so
  dropping them one at a time would leave a partially-parsed file claiming full
  confidence. The node is still ranked and returned — only its confidence claim
  is withheld.

**Source-lane graph expansion** (`rank_source/expand.rs::expand_from_seeds`, run
on the channel's scored list before it is cut to `MAX_SOURCE_CANDIDATES`). The
strongest `MAX_EXPANSION_SEEDS = 5` candidates holding an `ExplicitId`,
`ExactPath`, `ExactSymbol` or `StageDependency` rung become seeds. Their
neighbours along `Calls`, `Implements`, `Extends` and `References` edges, in
either direction, resolved and with edge confidence at least
`MIN_NEIGHBOR_EDGE_CONFIDENCE = 0.5`, are examined strongest edge first, at most
`MAX_EXAMINED_NEIGHBORS_PER_SEED = 32` per seed. A neighbour is accepted only if
it is not already a candidate, is not a `File` node, and has `FileCoverage::Full`
— at most `MAX_NEIGHBORS_PER_SEED = 4` per seed and `MAX_EXPANDED = 12` in all.
An accepted neighbour carries the single reason `GraphNeighbor`,
`matched_term_count` 0 and a `Confidence::Medium` ceiling; its score is
`seed score × NEIGHBOR_SCORE_FACTOR` (0.2), capped at
`BOOST_EXACT_SYMBOL × test_path_factor` (32 at the defaults) and then put through
the test-path factor, so a neighbour can never outrank a direct exact hit, even
one in a test file. `GraphNeighbor` is not an exact rung, so neighbours fuse in
tier 2; the prompt hook admits one only while the pack still holds an exact-rung
item (see *Brief Delivery* below).

Packing dispatches on `candidate.channel`, never by trying both maps
(`pack.rs::build_item`). A source item is `ItemKind::SourceNode` with
`id = node.id` (`<path>#<kind>:<scope>`, disjoint from a knowledge chunk id),
`content_hash = node.body_hash`, and an excerpt taken from the signature. The
strict dispatch is deliberate: if the two id spaces ever did collide, this
surfaces it as a bug instead of silently masking it.

## Exact-Rung Gating: the Query Side Also Has to Look Like Code

Before A.1, boundary-checked substring matching (`contains_whole_term`) fixed
only the *candidate* side of an exact match — a symbol named `n` stopped
matching every prompt containing the letter — but nothing checked whether the
*query occurrence* looked like a code reference at all. Measured, all real:
"why doesn't loom repair --fix do it, **the point** is" pulled in `lerpPoint`,
`repairGini` and `type Point` at `high` confidence purely because an ordinary
English word happened to equal a symbol name.

`lexical::ExactGate` (`context/lexical/evidence.rs`) now admits a rung only
when the occurrence carries one of three independent signals, any one being
sufficient:

- **backticked** — inside a `` `…` `` span in the raw prompt;
- **shaped** — the matched name is identifier-shaped: contains `_` or `::`,
  or has an interior lowercase→uppercase transition (camelCase). A leading
  capital alone (`Point`, `Widget`) does NOT count — those are ordinary
  English words Rust happens to capitalize;
- **rare** — the name's document frequency in THIS channel's corpus is at
  most `config.df_ident_max` (default `5`), AND the name is not spelled like
  an ordinary word. A name of nothing but lowercase ASCII letters is refused
  outright (`evidence.rs::is_plain_word`, applied in `ExactGate::admits`):
  rarity cannot tell an uncommon symbol from an uncommon English word, and
  `remaining`, `relevant` and `complete` are all rare in a corpus of function
  signatures. `Foo::Bar`, `sha256` and `Gini` still qualify — a digit or a
  capital anywhere is a spelling the writer did not have to use. A name
  absent from the frequency map counts as rare — the map holds every
  tokenized query term, so an absent name is one no query token equals at all.

A full relative path still fires unconditionally — a path in a prompt is
always deliberate (`rank_source.rs::matches_path`, `PathMatch::FullPath`).

A rung admitted on **rarity alone** is capped at `Confidence::Medium`, never
`High`, via `rank::rungs::RungScore::confidence_ceiling()` — one full-strength
rung (backticked or shaped) is enough to restore `High` even alongside a
weaker one. The ceiling is published, not just computed: `pack` builds every
item through `RankedCandidate::confidence()`, which returns the WEAKER of the
reasons-implied confidence and the ceiling — never `Confidence::from_reasons`
directly (`rank.rs:144-178`, `pack.rs:96-100,154-156`). See
[Tests That Cannot Fail](../mistakes/tests-that-cannot-fail.md) for why a
ranker-level test alone did not catch a packer that forgot this.

Nothing here excludes a candidate from the KNOWLEDGE channel — a word that
fails every test still competes on its BM25 score, it just cannot buy the
~80-point exact-symbol boost with a coincidence. The SOURCE channel has a
second floor one level down, at plain lexical candidacy
(`rank_source/candidacy.rs::admits_lexical_evidence`): a source node that
earned no rung is a candidate only when every word of its terminal scope
segment is a surviving query term and that name has more than one word.
Required ids are exempt, and a one-word name falls back to the gate above, so
`` `tokenize` `` still reaches `fn tokenize` while `what does tokenize do` no
longer does. The reason this floor is needed on one channel and not the other
is per-channel stopwording asymmetry: source documents are ~10-token scope and
signature strings holding almost no prose, so the project vocabulary the
knowledge channel drops as ubiquitous — `hooks`, `sessions`, `settings` —
survives in the source corpus and is the most discriminating thing there.
Measured, that let `fs/permissions/hooks.rs#function:configure_loom_hooks` win
tier 2 of "how do I configure the hooks so sessions get the right settings" on
`configure` and `hooks` alone.

## Corpus-Derived Query Stopwording, With a Rescue Floor

Query terms are stopworded against the SAME corpus the channel ranks against,
not a fixed English list — a fixed list catches "the" and "is" and stops
there, while the words that actually flood this retrieval are the project's
own ("loom", "stage", "signal", "context"). A term is dropped when its
document frequency exceeds `corpus_size * stop_df_ratio` (default `0.10`) or
it is shorter than `min_query_token_len` (default `3`), UNLESS it occurs
backticked in the raw prompt. A chunk or node is a candidate only if it earned
a rung or matched a surviving term (`rank/corpus/stopwords.rs::partition_terms`).

**The rescue floor exists because indexing prose changed what "ubiquitous"
means.** Measured on this repository: on the curated-only corpus (658 docs,
floor 65.8) the query "worktree claude code sandbox settings rules sessions"
kept `settings`=57, `rules`=48, `sessions`=65 and returned a pack. Once A.15
indexed project prose into the same corpus (904 docs, floor 90.4) those same
terms inflated to 105, 93, 102 — prose is loom's own design docs, sharing the
question's vocabulary — so EVERY term exceeded the floor and an ordinary
question about the codebase returned an empty pack. Up to `RESCUE_LIMIT = 3`
of the rarest dropped terms are put back, subject to a hard
`stop_rescue_max_ratio` ceiling (default `0.25`): a term at 11% of the corpus
comes back when the query needs it; a term at 90% never does, so a genuine
stopword-only query still retrieves nothing. Rescued terms are removed from
`dropped_terms`, which stays a truthful account of what was actually dropped
(`stopwords.rs::rescue_rarest`).

**The floor is under a THIN surviving set, not only an empty one**
(`stopwords.rs::over_stopworded`). The first version fired only when
stopwording dropped every term, which left the measured query "sandbox
settings rules for claude code worktree sessions" answered on `rules` alone:
seven of its eight content words were above the floor, the one generic word
that survived counted as a survivor, and the expected chunk in
[Sandbox and Settings](../mistakes/sandbox-and-settings.md) ("Worktree
Settings Are a Whole-Object Rebuild") never says "rules", so it was not a
candidate at all. The rescue now
also fires when a query of at least `RESCUE_QUERY_MIN_TERMS = 4` distinct
content terms kept fewer DISTINCT surviving terms than `min_knowledge_terms`
(default `2`). Both constants are chosen, not tuned:

- the survivor floor REUSES `min_knowledge_terms` because that is the prompt
  hook's emit floor (`user_prompt_compose::clears_emit_floor`) — below it no
  purely lexical item can ever be emitted, so a query reduced under it has
  retrieved nothing whether or not one word is still standing. An operator who
  lowers the floor to 1 therefore turns the thin-survivor rescue off;
- `RESCUE_QUERY_MIN_TERMS` keeps short prompts out. "Honesty Contract" left
  with one surviving term was served well — that survivor IS the lookup — and
  putting its neighbours back would only add noise. Four or more content words
  is a question, and a question reduced to one generic word was answered on
  the least of what it asked.

The cap stays flat at three on both paths; sizing it to the deficit instead
(enough to reach `min_knowledge_terms`, no more) was measured and rejected,
because on the case above it restores `sessions` alone rather than `sessions`,
`settings` and `sandbox` together. Note what the fix does and does not buy:
that chunk went from not-a-candidate to a candidate ranked around 28th, and
the rescued `sandbox`/`settings` also feed the source channel's candidacy
rule, so two `sandbox_settings` source nodes now enter the fused top 5. The
`genuine-win-sandbox-settings-rules` eval case still misses at hit@5 — what is
left there is a ranking question, not a candidacy one.

One document-frequency map serves both stopwording (drops the ubiquitous) and
`ExactGate::is_rare` (admits the rare) — deliberately: excluding prose from
the frequency statistics was considered and rejected, because it would let a
term be simultaneously "too common to score" and "rare enough to claim an
exact symbol match" by two disagreeing counts. Dropped terms stay in the
frequency map for exactly this reason.

## Two-Tier Fusion (Not Reciprocal-Rank Fusion Alone)

`fuse` (`context/fuse.rs`) used to be plain reciprocal-rank fusion (RRF),
which reduces every candidate to its rank *position* — so a knowledge chunk
scoring 1080 on an explicit-id hit and a source node scoring 0.3 on a weak
lexical match both landed at rank 1 and tied exactly, falling through to
alphabetical id order. That systematically ordered the fused head by path
prefix rather than by relevance.

Fusion is now two tiers:

- **Tier 1** holds every candidate (from either channel) whose reasons
  include at least one exact rung — `ExplicitId`, `ExactPath`, `ExactSymbol`,
  `LinkedFrom` or `StageDependency`. Classification runs on the MERGED reason
  set across channels, so a candidate that is exact-rung in one channel and
  lexical-only in another is still tier 1 everywhere. Ordered by raw score
  descending (the max raw score seen across channels, never the sum — summing
  would reward a candidate merely for appearing twice), then id ascending, and
  precedes ALL of tier 2.
- **Tier 2** holds the remainder. Each channel's survivors (tier-1 ids
  removed) are renumbered from 1 and fused by ordinary RRF (`RRF_K = 60`,
  unchanged). Ties in RRF score break by **within-channel normalized score**
  (`raw_score / that channel's max raw score`, `0.0` on a zero or
  non-finite divisor) descending, then id ascending.

Tier-1 raw scores are comparable ACROSS channels only because both rankers
import the same boost constants (`BOOST_EXACT_PATH`, `BOOST_EXACT_SYMBOL`,
`BOOST_EXPLICIT_ID` from `rank.rs`) — a `100.0` exact-path hit means the same
thing whichever channel produced it. **`score` is NOT comparable across the
tier boundary**: tier 1 is raw ladder score (tens to 1000+), tier 2 is RRF
score (roughly `1/RRF_K` and smaller). Downstream readers of `score` —
`pack::build_omission_summary`'s `weakest_included_score` and the hook's
`without_weakest` — happen to behave correctly today because the weakest
item is always tier-2, a consequence of the scale gap, not something either
function checks (the `fuse` module doc).

A source-lane `GraphNeighbor` candidate never holds an exact rung, so it always
fuses in tier 2. Packing the fused list is the next section's subject.

## Packing: Required Items, Rendered Cost and Excerpts

`pack` (`context/pack.rs`) turns the fused list into a `ContextPack` in two
passes, and **there is still no per-channel budget**: a query that matches prose
strongly can legitimately fill the pack with prose. A source item costs tens of
tokens against a knowledge chunk's hundreds, so an unbounded source list crowds
prose out by *slot*, not by token volume.

**Required items first** (`pack/required.rs::reserve_within_budget`). Every
candidate carrying `ExplicitId` — a `--require-id` — is weighed before any
optional one, in the request's `RequiredRepresentation`: `Full` by default (the
whole chunk body or node signature, never truncated), or `Compact` with
`loom knowledge context --require-compact` (the bounded excerpt, marked
`truncated` when cut). One that does not fit becomes an
`UnmetRequirement { id, needed_tokens, available_tokens, reason }` in
`ContextPack::unmet_required`, rendered as a
`Required but unmet: <id> (needs ~N tokens, M available)` line, and
`loom knowledge context` then exits 3. Because an unmet line costs tokens too,
the reservation re-runs with the measured unmet cost held back until the result
fits — at most one extra pass per required candidate — and admitted items are
never evicted to make room for a line. A pack comes out over budget only when the
frame plus the unmet lines alone exceed it; `items` is then empty and the
estimate is pinned to that floor.

**Then optional items** (`select_optional`), walked in fused order, each built
`Compact` and taken only if the brief it would produce still fits. A candidate
with no backing chunk or node, one that does not fit, and a superseded tier-1
twin (below) are all counted in `OmissionSummary::omitted`.

**Rendered cost, not raw size** (`context/render.rs`). An item's `token_count` is
`rendered_item_tokens` — the cost of the text the brief will actually print for
it — and every fit decision prices the whole brief through `tentative_total` →
`rendered_brief_tokens` = `BRIEF_FRAME_TOKENS` (128, the brief's header and
footer) + every item's rendered cost + `rendered_chrome_tokens` (the
`### Knowledge` and `### Source (signature index)` headings, the per-path source
bullet prefixes and joiners, and the unmet lines). `ContextPack::recompute_estimate`
calls the same function, so a budget decision and the published
`estimated_tokens` cannot disagree; `within_budget` is
`estimated_tokens <= budget_tokens`. `MIN_BUDGET_TOKENS` is
`BRIEF_FRAME_TOKENS + 128`, so a configured budget always leaves room above the
frame.

**Match-centred excerpts** (`pack/excerpt.rs::bounded_excerpt`). A body within
`EXCERPT_MAX_TOKENS` (400 estimated tokens, 1 600 bytes at
`BYTES_PER_TOKEN_ESTIMATE = 4`) is quoted whole. A longer one is windowed around
its best-matching line for the surviving query terms: that whole line is always
inside the window (a single over-long line becomes the window and is charged
honestly); the remaining budget is spent half before and half after it,
line-aligned, with whatever one side cannot use going to the other;
`[… earlier lines omitted]` appears exactly when the window starts after the first
byte, and `EXCERPT_TRUNCATION_MARKER` exactly when it ends before the body does.
With no matching line the leading window is used. A window that starts or ends
inside a fenced code block has its fence re-opened, closed or trimmed so the quote
stays well-formed. `ContextItem::truncated` reports whether anything was cut.

**Tier-1 summaries never ride along with their tier-2 detail.** A tier-1 file keeps a 2-8 line summary per topic ending in a link to the topic file under the tier-1 file's stem directory, and `loom knowledge update` scaffolds that topic file with the tier-1 heading verbatim, so the pair shares an anchor and scores nearly identically on the same terms. `pack::twins::tier1_twin` maps a tier-2 chunk id to its tier-1 twin, keying strictly on the tier-2 file's parent directory equalling the tier-1 file's stem, so an unrelated pair that merely shares an anchor is never collapsed. `prose:` ids, deeper paths, empty anchors and source-node ids have no twin. While packing, `details_before_summaries` walks a detail immediately before the summary it duplicates, and `select_optional` drops the summary once the detail is packed, counting it in `OmissionSummary::omitted`. When the detail does not fit the budget the summary is packed as before, which is what makes it a fallback rather than a deletion. A summary the caller named through `--require-id` is exempt from both halves: the request is answered literally, and its `ExplicitId` boost is not allowed to promote a weakly-ranked detail to the head of the pack.

## Persistent BM25 Index (A.13)

Every prompt used to re-tokenize the whole corpus from scratch — ~656
knowledge chunks and ~7,900 source nodes on this repository — then scan it
again per query term for document frequencies, inside a hook with a hard
five-second ceiling. A persistent inverted index (`context/lexical_index.rs`)
now makes a cache hit skip the tokenization entirely:

- Keyed per channel: the knowledge index by the catalog revision, the source
  index by `lexical_index::source_layer_key` — a hash of the resolved layer
  actually being indexed (base revision plus each file's path, content hash
  and parser version), NOT the overlay fingerprint, because the ranker never
  receives that. A key/corpus mismatch is structurally impossible rather than
  merely unlikely.
- **The full scan stays the default and the correctness oracle.** Every
  caller with no cache root — every existing test included — gets the scan;
  `rank::corpus::score_terms` is the ONE arithmetic implementation both the
  scanned and indexed representations route through, so a cache hit cannot
  score differently from a miss. A property test asserts indexed scoring
  equals scan scoring exactly (same scores, distinct-term counts, candidates,
  order) across randomly generated corpora.
- **Not stored:** `average_length` and the document-frequency map. Both are
  exact functions of what IS stored (document lengths, postings) and a
  persisted derived value is a second source of truth that can only be
  wrong. Recomputed on load by the same expressions the scan uses.
- **Weights are stored as raw IEEE-754 bits**, not decimal, so a round trip
  cannot shift a score by an ULP.
- The file hashes the `WEIGHT_*` constants (`derivation()`,
  `lexical_index.rs:85-98`) and is rejected — falls back to the scan, then
  rewrites — when they no longer match, so retuning a weight cannot leave a
  warm cache scoring at the old value.
- **`INDEX_VERSION` (currently `1`) must be bumped whenever `lexical::tokenize`
  changes** — the one input to a document with no constant to hash. See
  [conventions.md](../conventions.md).
- Pruning (`lexical_index/cache.rs`) keeps a bounded number of index files per
  channel (`KEEP_INDEXES = 6`) rather than unlinking every sibling revision:
  parallel worktrees resolve different keys against one shared cache
  directory, so unlink-all would have each stage evict every other stage's
  index on every prompt.
- Every write is best-effort and silent (`debug!`, never an error) — a
  sandboxed or read-only caller still retrieves.

## Indexed Prose: a Third Corpus Component (A.15)

A file with no registered tree-sitter grammar produced only a whole-file
node, and `rank_source` drops whole-file nodes — so design documents under
`doc/` were unreachable by retrieval even though `context::extract`'s own
docstring claimed otherwise. Every `*.md` under `config.prose_roots` (default
`["doc"]`) is now chunked by the same heading chunker the curated tree uses,
with every id PREFIXED `prose:` (`fs::knowledge::catalog::prose::PROSE_ID_PREFIX`)
so it can never collide with a curated chunk id and the prefix itself signals
origin. Completed plans (`DONE-` filenames under a `plans/` path segment) are
excluded as history; the curated knowledge tree itself is skipped during the
walk (it already has its own chunker) so `prose_roots = ["doc"]` does not
double-index every curated chunk as its own prose clone.

Prose participates in the structural (catalog) revision, so editing a design
doc marks the catalog stale and it re-indexes on the next query — one
function derives the prose source list for both the chunker and the
fingerprinter, so the two halves of the freshness contract cannot disagree.

**Curated knowledge keeps priority by DEMOTING prose, never by promoting
curated.** `rank::prose_demotion` subtracts `config.knowledge_curated_prior`
(default `5.0`, an increment applied after BM25 + rung scoring, not a
multiplier) from a `prose:`-prefixed candidate's score. Promoting curated
instead would have been equivalent for curated-vs-prose ordering but would
also inflate the knowledge channel against the source channel and compress
the within-channel normalized scores tier-2 fusion's tie-break depends on.
The demotion is **clamped at zero** — left unclamped, a query answered only
by prose gives the channel a negative maximum, and tier 2's
`raw_score / channel_max` INVERTS the ordering (`-3.0/-1.0 = 3.0` outranks
`-1.0/-1.0 = 1.0`), putting the worst match first. Applied AFTER the
candidacy check, not inside the exact-match ladder, so it never turns "no
rung fired" into a candidate — that would make every curated chunk a
candidate on every query and undo the stopwording candidacy floor.

The pack's `dropped_terms` is now the UNION of both channels' drops, not
whichever channel was consulted first — with per-corpus ubiquity floors the
two channels genuinely differ on what they drop.

## The Pipeline

```text
knowledge/*.md ──chunker──┐
doc/**/*.md (prose) ──────┼──> KnowledgeChunk ──catalog──> Catalog (revision)
                           │                                    │
                       fingerprint ──> Freshness              ingest
                           │                                    │
                           └──────────> store (.loom/cache/context-v1/) <┘
                                              │
      rank (+ graph expansion) ──> lifecycle filter ──> fuse (two-tier) ──> pack ──> ContextPack
         ▲
 lexical_index (per-revision cache; scan is the oracle)
```

- **One evaluate per retrieval.** `retrieve_for_stage` → `resolve_catalog` →
  `refresh(store, knowledge_root, structural_only = true)`, which runs
  `evaluate_inner` exactly once — one fingerprint pass — and hands those
  fingerprints to the rebuild when the catalog is stale; this path never rebuilds
  the semantic layer. The standalone `evaluate_state` runs only when that refresh
  fails (an in-memory catalog is built instead) or when there is no knowledge
  tree. A stored semantic revision behind `HEAD` is reported stale, and
  `retrieve::graph::load_resolved_graph` also marks it stale when the working
  tree's generation no longer matches the overlay being read.
- `rank` scores each requested channel independently, through the shared BM25 +
  exact-rung machinery described above; the source channel adds its graph
  neighbours before truncation.
- **Lifecycle eligibility** (`retrieve/channels.rs::apply_lifecycle_policy`,
  knowledge channel only): under the default `LifecyclePolicy::Current` only
  `Active` and `Draft` chunks are candidates; `loom knowledge context --history`
  switches to `LifecyclePolicy::Historical`, which also admits `Deprecated`,
  `Superseded` and `Historical`. An `ExplicitId` candidate is always admitted. A
  curated chunk's state comes from its file's frontmatter `state`
  (`loom knowledge annotate --state`), default `Active`. An indexed prose file's
  comes from its path (`catalog/prose.rs::prose_lifecycle`): under a `plans`
  directory, `DONE-` files are not indexed, `REVIEW-` files are `Historical`, and
  `PLAN-` / `IN_PROGRESS-PLAN-` files and anything under `briefs` are `Draft`;
  anywhere, `REPORT-` / `PROPOSAL-` files are `Historical` and anything under an
  `archive` directory is not indexed; an explicit frontmatter state overrides the
  path-derived one. Source nodes are always `Active`.
- `fuse` merges the per-channel lists by **two-tier fusion** (exact-rung
  candidates first by raw score, the lexical remainder by RRF) — see above.
- `pack` reserves required items, then takes optional items in fused order as
  compact, match-centred excerpts while the rendered brief fits, and always
  reports what it left out (`OmissionSummary`, `unmet_required`) — see *Packing*
  above.

**Budgets come from config.** `RetrievalConfig::load` reads the `[retrieval]`
table of the main project root's `.loom/config.toml`: the stage spawn brief
(`orchestrator/signals/retrieval.rs::retrieve_stage_pack`) is packed to
`stage_brief_budget_tokens` (default 3000) and the prompt hook to
`prompt_budget_tokens` (default 1500); an out-of-range value is clamped rather
than rejected. `loom knowledge context --budget-tokens` defaults to 2000.

**One entry point.** `retrieve::retrieve_for_stage` runs the whole pipeline
and is the only way in — `loom knowledge context`, `loom knowledge eval`
(the retrieval evaluation harness, below), signal generation and the prompt
hook all call it, so a brief rendered at spawn time and a brief pulled by
hand are built the same way (the `context` module doc). Adding a fifth
consumer means calling that function, not reimplementing the pipeline.

**Retrieval evaluation harness (A.20).** `loom knowledge eval` (dispatched from
`cli/dispatch.rs::dispatch_knowledge`, implemented in `commands/knowledge/eval.rs`)
scores a checked-in case file (default `loom/eval/retrieval-cases.yaml`) through
`retrieve_for_stage` against the LIVE on-disk index and reports per-case
hit@5/MRR plus aggregate precision@5, exiting non-zero when aggregate
precision falls below the file's `pass_floor` or any `forbid` id appears
anywhere in a case's results. `forbid`-only cases are excluded from the
precision denominator so a fixed regression case cannot cap the score
forever, and a case with neither `expect` nor `forbid` fails construction —
it could never fail the run. Deliberately NOT wired into `cargo test`: it
reads the live index, which is not reproducible in CI. Its CLI help now also
lists mandatory recall, abstention and rendered cost among the reported metrics
(`commands/knowledge/eval/metrics.rs`); the metric description above predates that
change. `scripts/harvest-eval-cases` drafts further cases from session transcripts
for hand-labeling. `scripts/retrieval-ab` measures precision@5, injected tokens per
brief and hook wall-time percentiles against a baseline binary, routed through one
env-stripping helper so its "isolated" measure root cannot inherit the
calling session's `LOOM_WORK_DIR` (see
[Never Spawn a Surviving Process From a Test](../mistakes/detached-spawn-in-tests.md)
for why an inherited env var made an "isolated" harness mutate the real
checkout).

## Base vs Overlay Ownership

This is the rule that keeps parallel worktrees from corrupting each other (the
`graph_store` module doc). Parallel stages run in separate worktrees off one
repository; if they shared a mutable graph a stage would see HALF of a sibling's
edits — worse than seeing none, because there is no way to tell which half.

| Layer | Location | Keyed by | Mutability |
| --- | --- | --- | --- |
| **base** | `.loom/cache/context-v1/graph/base/<revision>.json` under the canonical MAIN project root, shared by every worktree | the commit it describes | written once, thereafter immutable; old bases are pruned only when a new one is published |
| **overlay** | `graph.json` in `.loom/work/context/<plan>/<stage>/` | plan + stage — a real stage, or `_local` / `map-<dir>` for a checkout's working tree | rewritten by its owner; holds only the files that differ from the base |

A read is `overlay ∪ (base − overlay's files)`. An overlay entry shadows the base
entry for the same path **wholesale, never merges with it** — partial merges
produce a graph that describes no revision that ever existed — and an overlay
tombstone (`FileCoverage::Deleted`) removes the path from the view entirely. The
known gap an earlier version of this section recorded, that an overlay could not
express a deletion so a deleted file kept its base outline, is closed; see
[Source Graph](source-graph.md) for how tombstones are built and filtered.

`graph_store` owns only the layout, the layering rule and canonical
serialization. It never builds a graph (`context::refresh` does) and never
decides *when* to write one (`refresh::ensure_snapshot` and
`reconcile_source_graph` do).

**Which overlay a query reads** is an `OverlayScope`: the stage spawn brief reads
its stage's own overlay (`OverlayScope::Stage`, plan from `delivery::plan_key`);
the CLI reads `OverlayScope::Local`; the prompt hook reads whatever its
`HookTarget` resolved — the stage overlay inside a stage, `Local` in a plain
checkout.

**A missing base is not automatically a degraded pack.** `GraphStore::resolved`
substitutes an empty base when no base file exists for the recorded semantic
revision. Bases are built from committed `HEAD` content even in a dirty checkout
(`ensure_snapshot`), with working-tree changes carried by the `_local` overlay, so
a missing base usually means nothing has published one for this `HEAD` yet.
`ContextPack::degraded` (A.11) fires only for the narrower case: a non-empty
semantic revision that NEITHER the base nor any overlay can back at all, so the
resolved graph has no content whatsoever (`retrieve/graph.rs::degraded_reason`).
Widening that predicate to "any missing base" was tried and reverted — it flagged
every healthy checkout as degraded permanently, and
`reconcile_graph::spawn_if_needed` triggers on `stale OR degraded`, so it also
started a detached full-repository tree-sitter rebuild on every single prompt in
every working checkout. See `degraded_reason`'s own doc comment for the
reconcile-trigger consequence before widening this predicate again. Separately,
the read marks the semantic layer stale — without degrading the pack — when the
overlay's `generation` no longer matches the working tree, or the tree is dirty and
no overlay exists; that too wakes the background reconcile.

## Derived vs Durable

Getting this wrong destroys work, so it is worth stating flatly:

- **Derived / regenerable:** everything under `.loom/cache/context-v1/` (chunk
  catalog, fingerprints, base graph layers, the persistent lexical index).
  Safe to delete; `loom knowledge sync` rebuilds it. It is git-ignored.
- **Durable within a run:** the per-stage overlay and the **delivery records**
  under `.work/context/<plan>/<stage>/`. These are NOT regenerable from the
  repo alone — a delivery record states what a specific recipient was already
  given.
- **Durable forever:** only `doc/loom/knowledge/*.md`, the curated prose itself
  (indexed prose under other `doc/` paths is durable too, but it is source
  documentation with its own reason to exist, not knowledge-base content).

The distinction has already caused one 100%-reproducible defect: a discard
routine deleted delivery records out of a directory shared with the graph layer,
so the dependency-ranking boost failed every time on the daemon path. The fix
was to discard only the graph layer, not the shared directory (commit
`7e35eef7`). Rule: **a "discard the derived layer" operation must name the layer,
never the directory** — check what else writes into that directory first.

## Delivery Records and Epoch Suppression

`context/delivery.rs` answers "has this recipient already been given these exact
bytes?", so a second retrieval in the same session can skip what the first
already quoted instead of repeating it.

- The record is an **optimisation, never state the run depends on**. Nothing in
  it may fail a spawn or a hook: a missing directory reads as "nothing
  delivered", and an unreadable or malformed file is skipped rather than
  propagated (the `delivery` module doc).
- Suppression is scoped to a **`context_epoch`**: once a derived layer is
  rebuilt the same id may describe different bytes, so every record from an older
  epoch is ignored and delivery re-opens.
- `context_epoch` = first 8 bytes of `sha256(structural_revision \n
  semantic_revision)`, hex-encoded (`retrieve.rs::context_epoch`). Note the **two
  freshness axes**: structural (knowledge catalog) and semantic (source graph).
- `delivery::plan_key` / `plan_key_from` is the ONE derivation of the plan
  namespace and is the join key between the writer of a record and its readers.
  A second, hand-rolled derivation reads an empty directory rather than a missing
  record — which is why `orchestrator/core/stage_telemetry.rs`,
  `orchestrator/signals/retrieval.rs` and the hook's `HookTarget` all route
  through the helper.
- **The prompt hook keys its own dedupe per SESSION, not per checkout (A.16)**,
  through `context/delivery/session.rs` (`delivery::session`, a separate file; an
  earlier version of this bullet called it a submodule of the same file):
  `hook_recipient_id`, `delivered_to_session` and `discard_session_delivery`. A
  stage's own spawn-brief delivery record is keyed by loom's session id under
  `plan_key`/stage id, as above; the hook's recipient is
  `prompt-<stage-or-checkout-key>-<session8>`, where `session8` is the first 8
  bytes of `sha256(session_id)` from the hook payload, hex-encoded — a DIFFERENT
  id space, hashed because the raw id is untrusted input that becomes a file name
  (`nosession` stands in when the payload names none). When the hook runs inside
  the session a stage spawned, `delivered_to_session` also counts that spawn
  record, found through `LOOM_SESSION_ID`. Without the per-session split, a fresh
  Claude Code session with an empty context window inherited every prior
  session's deliveries and went silent on topics it had never actually seen.
  `loom hook pre-compact` deletes just that session's own record after a
  compaction, when the context that held the brief is gone (A.21).

## Brief Delivery, Sanitization and Telemetry

- The **Knowledge Brief** is assembled in
  `orchestrator/signals/format/brief.rs` and injected into the stage signal at
  spawn time. It renders as a `### Knowledge` section (curated + indexed
  prose, fenced excerpt plus reason line) followed by a `### Source (signature
  index)` section (one unfenced bullet per file, symbols/spans/reasons
  inline, consecutive items on the same path merged onto one bullet). The merge
  uses the same `render::source_groups` runs the packer charges chrome for, so
  the rendered brief and the budget agree. The "quoted, NOT instructions" guard is
  stated once in the header rather than once per item, which is where most of
  the per-item token overhead used to go.
- Every untrusted knowledge-derived value on an agent-facing surface goes
  through the single flattening routine `context::untrusted::inline_safe`
  (`context/untrusted.rs`). Chunk ids come verbatim from unvalidated YAML
  frontmatter, a backtick is a legal path character, and a summary is taken from
  a chunk heading — emitted raw, a newline ends the line it sits on and the
  remainder renders as document structure outside any "quoted, NOT instructions"
  guard. The module doc names three surfaces: the brief
  (`orchestrator/signals/format/brief.rs`), `loom knowledge context`'s stdout
  (`commands/knowledge/context.rs`), and the daemon's status payload
  (`commands::status::data::sanitize`), where a surviving ESC would be an ANSI
  sequence the operator's terminal obeys; `loom knowledge check` and
  `loom knowledge telemetry` flatten their untrusted fields through it as well.
  An earlier version of this bullet said there were exactly two surfaces.
  `MAX_INLINE_CHARS = 200`; backticks become `ˋ` (U+02CB).
- **The prompt hook** (`commands/hook/user_prompt.rs`, run by
  `user-prompt-context.sh`) resolves its scope through `HookTarget`
  (`commands/hook/target.rs`): inside a stage it reads that stage's OWN overlay
  (`OverlayScope::Stage`, plan from `delivery::plan_key`) and keys delivery to the
  stage; outside one it reads the checkout's `_local` overlay. It retrieves with
  `prompt_budget_tokens`, then applies its gates in order. `parse_prompt` declines
  machine-generated payloads before retrieval runs (task-notification XML, and the
  "Background agent" and "Caveat:" prefixes), strips `@` file attachments, and
  requires 24 characters of question. **Per-item admission** then drops every
  item that does not clear the floor on its own — an exact-rung reason, or a match
  on at least `config.min_knowledge_terms` (default `2`) distinct surviving terms,
  for ANY item, not only knowledge chunks — except a `GraphNeighbor` item, which is
  admitted while the retrieved pack still holds an exact-rung item; dropped items
  fold into `omitted`. Next the session's per-epoch dedupe drops what this session
  was already handed, and finally the payload must fit `config.max_payload_bytes`
  (default `16384`), shedding the weakest item until it does. The hook
  **abstains** — prints nothing — with reason `floor` (nothing admitted, or what
  survives dedupe no longer clears the floor), `all-delivered`, or
  `over-ceiling`; a retrieval error is `no-retrieval` and an unresolvable
  environment `no-target`. The floor applies to UNSOLICITED injection only —
  `loom knowledge context`, `loom knowledge eval` and the stage spawn brief are
  deliberately not gated. After printing, or abstaining on a retrieved pack, the
  hook nudges the detached source-graph reconcile when the pack is stale or
  degraded.
- **Telemetry** (`loom/src/telemetry/mod.rs`) appends one JSON line per event to
  the state directory's `telemetry/events.jsonl`; a sandboxed session whose
  direct write is denied spools to its worktree's `.loom/telemetry-spool.jsonl`,
  which the daemon drains alongside the memory spool. Events: `context-delivered`
  / `context-unavailable` (one per spawned session, from
  `stage_telemetry::record_context_telemetry` after the stage executor spawns it),
  `prompt-brief` / `prompt-abstained` (the prompt hook), and `context-pulled`
  (`loom knowledge context`). Best-effort by contract — `emit` never fails a
  caller, and `read_events` skips a malformed line. Counts are ITEM and
  estimated-token counts, never a saving. `loom knowledge telemetry` reads them
  back per stage (`telemetry::summary::summarize`), and `archive_run_state` copies
  the telemetry directory under `.loom/memory/archive/` before a finished plan's
  state directory is removed. An earlier version of this bullet said the events
  had no reader and were deleted at plan finalization.

## Layering

`context` depends only on `crate::context`, `crate::fs`, `crate::language`,
`crate::models`, one `crate::utils::truncate_for_display` call (inside
`context::untrusted::inline_safe`) and — deliberately — `crate::git`:
`git::runner::run_git_checked` in the three files under `refresh/source_graph/`
(`enumerate.rs`, `generation.rs`, `layer.rs`), needed to list tracked and
untracked files, read committed blobs and fingerprint the working tree. There is
**no** production edge up to `orchestrator`, `commands` or `daemon`: the one
`crate::orchestrator` import is in the test module `context/tests/overlay_key.rs`,
and `crate::commands` appears only in doc comments. Verified with
`rg -o 'crate::[a-z_]+' loom/src/context`, excluding tests and doc comments. Keep
it that way: the orchestrator calls into `context`, never the reverse. An earlier
version of this section put the single `git` edge in `refresh/source_graph.rs`
itself.

//! `loom knowledge context` — deterministic, offline context retrieval.
//!
//! Presentation only. The retrieval itself lives in
//! [`crate::context::retrieve::retrieve_for_stage`], which every other consumer
//! of context also calls; this module turns a CLI invocation into a
//! [`StageQuery`] and renders the resulting [`ContextPack`]. There is no model
//! call and no network access anywhere in this path.

use crate::context::delivery::dependency_chunk_ids;
use crate::context::local_overlay::OverlayScope;
use crate::context::render::render_excerpt_block;
use crate::context::retrieve::{resolve_roots, retrieve_for_stage, StageQuery};
use crate::context::store::ContextStore;
use crate::context::untrusted::inline_safe;
use crate::context::{
    Channel, Confidence, ContextItem, ContextPack, Freshness, ItemKind, LifecyclePolicy,
    OmissionSummary, RequiredRepresentation, SelectionReason, UnmetRequirement,
};
use crate::fs::work_dir::WorkDir;
use crate::verify::transitions::load_stage;
use anyhow::{bail, Context, Result};
use colored::Colorize;
use std::path::{Path, PathBuf};

/// The directory the CLI resolves the state directory and the knowledge tree from.
const WORK_DIR_HINT: &str = ".";

/// Resolve the knowledge root and the derived-artifact store together, so the
/// three context commands can never disagree about which tree or cache they use.
pub(super) fn resolve() -> Result<(PathBuf, ContextStore)> {
    resolve_roots(Path::new(WORK_DIR_HINT))
}

/// Parse `--scope` into the channels it names.
fn parse_scope(scope: &str) -> Result<Vec<Channel>> {
    match scope.to_ascii_lowercase().as_str() {
        "knowledge" => Ok(vec![Channel::Knowledge]),
        "source" => Ok(vec![Channel::Source]),
        "all" => Ok(Channel::all().to_vec()),
        other => bail!("Invalid --scope '{other}': expected one of knowledge, source, all"),
    }
}

/// Chunk ids `--stage` contributes to the query: those already delivered to
/// the stages it depends on.
///
/// A stage's dependencies are the work it was written to build on, so the
/// chunk ids already delivered to them are the strongest structural hint
/// available about what this stage's retrieval should favour. [`RankQuery`]'s
/// `stage_dependency_ids` is matched against chunk ids
/// (`crate::context::rank`), not stage ids, so the dependency *stage ids*
/// themselves would never match anything.
///
/// [`RankQuery`]: crate::context::rank::RankQuery
fn stage_dependency_ids(stage_id: &str) -> Result<Vec<String>> {
    let work_dir = WorkDir::new(WORK_DIR_HINT)?;
    let stage = load_stage(stage_id, work_dir.root())
        .with_context(|| format!("Failed to load stage '{stage_id}' named by --stage"))?;
    // A stage record predating plan-id tracking, or one loaded outside any
    // plan, falls back to the same "default" plan id `persist_delivery` (see
    // `orchestrator/signals/helpers.rs`) uses when recording its deliveries.
    let plan_id = crate::context::delivery::plan_key(&stage);
    Ok(dependency_chunk_ids(
        work_dir.root(),
        plan_id,
        &stage.dependencies,
    ))
}

/// Retrieve a token-budgeted context pack for `query`.
// Each parameter is its own CLI flag on `loom knowledge context`; collapsing
// them into an args struct would just move the flag-to-field wiring one
// level down without shrinking it.
#[allow(clippy::too_many_arguments)]
pub fn context(
    stage: Option<String>,
    query: String,
    budget_tokens: usize,
    scope: String,
    require_id: Vec<String>,
    history: bool,
    require_compact: bool,
    explain: bool,
    json: bool,
) -> Result<()> {
    // Validate the flags that cost nothing before touching the filesystem, so a
    // typo in --scope still reports itself rather than a stage-loading failure.
    let channels = parse_scope(&scope)?;
    let stage_dependencies = match stage.as_deref() {
        Some(stage_id) => stage_dependency_ids(stage_id)?,
        None => Vec::new(),
    };

    let stage_query = build_stage_query(
        query,
        require_id,
        stage_dependencies,
        channels,
        history,
        require_compact,
    );
    let context_pack = retrieve_for_stage(&stage_query, budget_tokens)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&context_pack)?);
    } else {
        print_human(&context_pack, explain);
    }

    if let Some(code) = exit_code_for(&context_pack) {
        std::process::exit(code);
    }
    Ok(())
}

fn build_stage_query(
    query: String,
    required_ids: Vec<String>,
    stage_dependencies: Vec<String>,
    channels: Vec<Channel>,
    history: bool,
    require_compact: bool,
) -> StageQuery {
    StageQuery {
        work_dir_hint: PathBuf::from(WORK_DIR_HINT),
        text: query,
        required_ids,
        lifecycle: if history {
            LifecyclePolicy::Historical
        } else {
            LifecyclePolicy::Current
        },
        required_representation: if require_compact {
            RequiredRepresentation::Compact
        } else {
            RequiredRepresentation::Full
        },
        stage_dependency_ids: stage_dependencies,
        // Dependency affinity is a stage-spawn signal: the CLI answers about
        // whatever the user typed, with no stage's file ownership behind it.
        dependency_paths: Vec::new(),
        scope: channels,
        // The CLI asks about the tree in front of the user, so it reads that
        // checkout's working-tree overlay — not the last clean base revision.
        overlay: OverlayScope::Local,
    }
}

fn exit_code_for(pack: &ContextPack) -> Option<i32> {
    (!pack.unmet_required.is_empty()).then_some(3)
}

fn print_human(pack: &ContextPack, explain: bool) {
    println!("{} {}", "Query:".bold(), pack.query);
    println!(
        "{} {}/{} tokens",
        "Budget:".bold(),
        pack.estimated_tokens,
        pack.budget_tokens
    );
    print_freshness_line("Structural", &pack.structural_freshness);
    print_freshness_line("Semantic", &pack.semantic_freshness);
    if let Some(line) = format_degraded(pack) {
        println!("{}", line.red().bold());
    }
    if explain {
        if let Some(line) = format_dropped_terms(pack) {
            println!("{line}");
        }
    }
    println!();

    if pack.items.is_empty() {
        println!("{}", "No items matched.".dimmed());
    } else {
        for item in &pack.items {
            print_item(item, explain);
        }
    }
    for requirement in &pack.unmet_required {
        println!("{}", format_unmet_requirement(requirement));
    }

    println!();
    print_omissions(&pack.omitted);
}

fn format_unmet_requirement(requirement: &UnmetRequirement) -> String {
    format!(
        "! required {} did not fit: needs ~{} tokens, {} were available (raise --budget-tokens or pass --require-compact)",
        inline_safe(&requirement.id),
        requirement.needed_tokens,
        requirement.available_tokens,
    )
}

/// The line naming the query terms retrieval dropped before scoring, or `None`
/// when it dropped none.
///
/// Shown under `--explain` only: a dropped term is diagnostic detail about why
/// a result set looks the way it does, not part of the answer.
///
/// Every term goes through [`inline_safe`] first. These are query-derived — the
/// user's own prompt text, or a stage's free-form metadata — rendered as this
/// command's own output on a surface an agent reads, which is exactly the shape
/// `untrusted`'s docstring describes.
fn format_dropped_terms(pack: &ContextPack) -> Option<String> {
    if pack.dropped_terms.is_empty() {
        return None;
    }
    let terms = pack
        .dropped_terms
        .iter()
        .map(|term| inline_safe(term.as_str()))
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "Dropped query terms: {terms} (corpus-ubiquitous or too short)"
    ))
}

/// The degradation banner, or `None` when the pack was built from a complete
/// index.
///
/// Printed with or without `--explain`. A pack served from a knowingly
/// incomplete index that renders identically to a healthy one is the failure
/// mode this whole surface exists to prevent: the reader concludes "there is
/// nothing to say" when the truth is "nothing was there to look at".
fn format_degraded(pack: &ContextPack) -> Option<String> {
    let message = pack.degraded.as_ref()?;
    Some(format!("DEGRADED: {}", inline_safe(message)))
}

fn print_freshness_line(label: &str, freshness: &Freshness) {
    let marker = if freshness.stale {
        "stale".yellow()
    } else {
        "current".green()
    };
    match &freshness.detail {
        Some(detail) => println!("{label}: {marker} ({detail})"),
        None => println!("{label}: {marker}"),
    }
}

/// Render one item's summary line, with every untrusted field flattened.
///
/// The id, path, anchor and summary are all untrusted: `fs::knowledge::chunker`
/// takes a chunk's id and heading verbatim from a file's unvalidated YAML
/// frontmatter (see `orchestrator::signals::format::brief`), and a path can
/// legally contain a backtick. Rendered raw, a newline in any of them would
/// let the rest of the value render as a new line of this command's own
/// output — including one shaped like a heading or an instruction — so each
/// goes through [`inline_safe`] before it reaches stdout. `item.score` is a
/// typed float, not free text, so it is left alone.
///
/// A confidence below High is named at the end of the line, the same rule the
/// Knowledge Brief renders by (`orchestrator::signals::format::brief`): a
/// score says how strongly an item ranked, never how much of that rank came
/// from a coincidence, so a Medium hit that reads identically to a High one
/// misleads on the one surface built to explain a result set. High says
/// nothing, because it is what most rows are.
fn format_item_line(item: &ContextItem) -> String {
    let anchor = if item.pointer.anchor.is_empty() {
        String::new()
    } else {
        format!("#{}", inline_safe(&item.pointer.anchor))
    };
    let confidence = match item.confidence {
        Confidence::High => String::new(),
        demoted => format!("  ({})", confidence_label(demoted)),
    };
    format!(
        "  {:>6.2}  {}  {}{}  {}{}",
        item.score,
        inline_safe(item.id.as_str()),
        inline_safe(&item.pointer.path.display().to_string()),
        anchor,
        inline_safe(&item.summary),
        confidence
    )
}

fn print_item(item: &ContextItem, explain: bool) {
    print!("{}", format_item_block(item, explain));
}

/// One item's full block: its summary line, its `--explain` reasons line when
/// asked, and — for a knowledge chunk carrying one — its excerpt, fenced the
/// same way the Knowledge Brief renders it, followed by the blank line that
/// separates it from whatever prints next. Returned as a string, the
/// convention every other renderer in this file follows, so callers only
/// print it.
///
/// A source-node item keeps its single line: `context::extract` never stores
/// a node's body (see `orchestrator::signals::format::brief`'s module docs),
/// so there is nothing to quote past the signature already on the line.
///
/// The excerpt is rendered through the SAME [`render_excerpt_block`] the
/// Knowledge Brief uses, so the escape-proofing has one implementation rather
/// than a second copy for this command. CLAUDE.md tells every agent "pull
/// instead of read": before this, the only way to see a chunk's quoted
/// content from this command was `--json`, which defeats that instruction by
/// forcing a full read of the raw pack.
fn format_item_block(item: &ContextItem, explain: bool) -> String {
    let mut out = format_item_line(item);
    out.push('\n');
    if explain {
        let reasons = item
            .reasons
            .iter()
            .map(SelectionReason::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!(
            "          {reasons} | confidence: {} | state: {}\n",
            confidence_label(item.confidence),
            item.state
        ));
        if item.truncated {
            out.push_str("          truncated: yes\n");
        }
    }
    if item.kind == ItemKind::KnowledgeChunk {
        if let Some(excerpt) = &item.excerpt {
            out.push_str(&render_excerpt_block(excerpt));
            out.push('\n');
        }
    }
    out
}

fn confidence_label(confidence: Confidence) -> &'static str {
    match confidence {
        Confidence::High => "high",
        Confidence::Medium => "medium",
        Confidence::Low => "low",
    }
}

/// `included_tokens` is the packer's rendered, excerpt-capped cost and
/// `candidate_tokens` is the ranker's whole-body estimate before packing —
/// see `pack::build_omission_summary`'s doc comment. They are not the same
/// unit, so printing them as one ratio would read as a fraction that isn't
/// one: a pack where every candidate fit could show e.g. "312/9000 tokens"
/// and look heavily incomplete. Each is labelled and printed on its own.
fn print_omissions(omitted: &OmissionSummary) {
    println!(
        "{} {} omitted (weakest included score: {:.2}) — {}/{} items, packed {} tokens (candidates estimated at {} tokens)",
        "Coverage:".cyan().bold(),
        omitted.omitted,
        omitted.weakest_included_score,
        omitted.coverage.included,
        omitted.coverage.candidates,
        omitted.coverage.included_tokens,
        omitted.coverage.candidate_tokens,
    );
}

#[cfg(test)]
#[path = "tests_context.rs"]
mod tests;

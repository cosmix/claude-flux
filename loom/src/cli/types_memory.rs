//! Memory and knowledge CLI command types

use crate::context::config::MIN_BUDGET_TOKENS;
use crate::validation::{clap_id_validator, clap_knowledge_content_validator};
use clap::Subcommand;
use std::path::PathBuf;

/// Clap value parser for `--budget-tokens`.
///
/// Rejects a budget below [`MIN_BUDGET_TOKENS`] outright rather than silently
/// clamping it up: a user who typed 50 should be told their budget cannot
/// even pay for the Knowledge Brief frame, not handed 256 with no explanation.
fn clap_budget_tokens_validator(s: &str) -> Result<usize, String> {
    let value: usize = s.parse().map_err(|_| format!("'{s}' is not a number"))?;
    if value < MIN_BUDGET_TOKENS {
        return Err(format!(
            "--budget-tokens must be at least {MIN_BUDGET_TOKENS} (got {value}); \
             a smaller budget cannot even pay for the Knowledge Brief frame"
        ));
    }
    Ok(value)
}

#[derive(Subcommand)]
pub enum KnowledgeCommands {
    /// Update (append to) a knowledge file
    Update {
        /// Tier-1 file (entry-points, patterns, conventions, ...) or tier-2 topic (<category>/<slug>)
        file: String,

        /// Content to append (markdown format). Omit or use "-" to read from stdin.
        #[arg(value_parser = clap_knowledge_content_validator)]
        content: Option<String>,
    },

    /// Replace a `#{2,6} <heading>` section in place, at whatever level it's found (corrects stale knowledge; appends if absent)
    ReplaceSection {
        /// Tier-1 file (entry-points, patterns, conventions, ...) or tier-2 topic (<category>/<slug>)
        file: String,

        /// Heading of the section to overwrite, with or without the leading `## `
        heading: String,

        /// Replacement body WITHOUT the heading line. Omit or use "-" to read from stdin.
        #[arg(value_parser = clap_knowledge_content_validator)]
        content: Option<String>,
    },

    /// Retrieve a token-budgeted context pack for a query (deterministic, offline)
    ///
    /// Exits with code 3 when a --require-id could not be honored.
    Context {
        /// Seed the query from this stage's dependencies, and name it in output
        #[arg(long)]
        stage: Option<String>,
        /// Query text to retrieve context for
        #[arg(long)]
        query: String,
        /// Maximum estimated tokens the pack may contain
        #[arg(long, default_value_t = 2000, value_parser = clap_budget_tokens_validator)]
        budget_tokens: usize,
        /// Retrieval channels to search: knowledge, source, or all
        #[arg(long, default_value = "all")]
        scope: String,
        /// Chunk id that must be included; repeatable
        #[arg(long = "require-id")]
        require_id: Vec<String>,
        /// Include deprecated, superseded and historical material (default: current knowledge only)
        #[arg(long)]
        history: bool,
        /// Represent --require-id items by their bounded excerpt instead of verbatim; the JSON marks them truncated
        #[arg(long)]
        require_compact: bool,
        /// Show per-item scores and selection reasons
        #[arg(long)]
        explain: bool,
        /// Machine-readable JSON output (suppresses human text)
        #[arg(long)]
        json: bool,
    },

    /// Score retrieval against a checked-in case file (precision@5 / MRR)
    Eval {
        /// Cases file; defaults to loom/eval/retrieval-cases.yaml under the project root
        #[arg(long)]
        cases: Option<PathBuf>,
        /// Override the per-case token budget
        #[arg(long, value_parser = clap_budget_tokens_validator)]
        budget_tokens: Option<usize>,
        /// Machine-readable JSON output (suppresses human text)
        #[arg(long)]
        json: bool,
    },

    /// Rebuild derived context artifacts, and upgrade a flat knowledge dir (creates INDEX.md)
    Sync {
        /// Rebuild only the structural (catalog) layer
        #[arg(long)]
        structural_only: bool,
        /// Machine-readable JSON output (suppresses human text)
        #[arg(long)]
        json: bool,
    },

    /// Report knowledge-base diagnostics (read-only; never opens the context store)
    Check {
        /// Exit non-zero when any issue is reported
        #[arg(long)]
        strict: bool,
        /// Machine-readable JSON output (suppresses human text)
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum MemoryCommands {
    /// Record a note in the stage memory
    Note {
        /// The note text
        text: String,

        /// Evidence reference (repeatable)
        #[arg(short = 'e', long)]
        evidence: Vec<String>,

        /// Stage ID (auto-detected from LOOM_STAGE_ID if not provided)
        #[arg(short = 'S', long, value_parser = clap_id_validator)]
        stage: Option<String>,
    },

    /// Record a decision with optional rationale
    Decision {
        /// The decision text
        text: String,

        /// Context or rationale for the decision
        #[arg(short, long)]
        context: Option<String>,

        /// Evidence reference (repeatable)
        #[arg(short = 'e', long)]
        evidence: Vec<String>,

        /// Stage ID (auto-detected from LOOM_STAGE_ID if not provided)
        #[arg(short = 'S', long, value_parser = clap_id_validator)]
        stage: Option<String>,
    },

    /// Record an open question
    Question {
        /// The question text
        text: String,

        /// Evidence reference (repeatable)
        #[arg(short = 'e', long)]
        evidence: Vec<String>,

        /// Stage ID (auto-detected from LOOM_STAGE_ID if not provided)
        #[arg(short = 'S', long, value_parser = clap_id_validator)]
        stage: Option<String>,
    },

    /// Record a file change
    Change {
        /// Description of what changed (e.g., "src/foo.rs - Added bar() function")
        text: String,

        /// Evidence reference (repeatable)
        #[arg(short = 'e', long)]
        evidence: Vec<String>,

        /// Stage ID (auto-detected from LOOM_STAGE_ID if not provided)
        #[arg(short = 'S', long, value_parser = clap_id_validator)]
        stage: Option<String>,
    },

    /// Record how a captured memory event was processed
    Resolve {
        /// ID of the note, decision, question, or change being settled
        event_id: String,

        /// Processing outcome
        #[arg(long, value_parser = ["promoted", "merged", "discarded", "deferred"])]
        outcome: String,

        /// Knowledge target for a promoted or merged event
        #[arg(long)]
        target: Option<String>,

        /// Explanation for the outcome
        #[arg(long)]
        reason: Option<String>,

        /// Stage ID (auto-detected from LOOM_STAGE_ID if not provided)
        #[arg(short = 'S', long, value_parser = clap_id_validator)]
        stage: Option<String>,
    },

    /// List notes, decisions, and questions that have no receipt
    Pending {
        /// Stage ID to scope pending entries to
        #[arg(short = 'S', long, value_parser = clap_id_validator)]
        stage: Option<String>,

        /// Print a machine-readable report
        #[arg(long)]
        json: bool,

        /// Exit non-zero when any pending entry is found
        #[arg(long)]
        strict: bool,
    },

    /// Search memory entries
    Query {
        /// Search term
        search: String,

        /// Stage ID to search (searches all if not provided)
        #[arg(short = 'S', long, value_parser = clap_id_validator)]
        stage: Option<String>,
    },

    /// List memory entries (all journals in the plan, or one stage with --stage)
    List {
        /// Stage ID to scope to (lists every journal in the plan if not provided)
        #[arg(short = 'S', long, value_parser = clap_id_validator)]
        stage: Option<String>,

        /// Filter by entry type (note, decision, question, change, receipt)
        #[arg(short = 't', long)]
        entry_type: Option<String>,

        /// Print entries as a JSON array
        #[arg(long)]
        json: bool,
    },

    /// Show full memory journal
    Show {
        /// Stage ID (auto-detected if not provided)
        #[arg(short = 'S', long, value_parser = clap_id_validator)]
        stage: Option<String>,

        /// Show ALL stage memories
        #[arg(short, long)]
        all: bool,

        /// Print entries as JSON (`--all` uses a stage-to-entries object)
        #[arg(long)]
        json: bool,
    },
}

//! Memory command implementations for managing stage memory journals.
//!
//! Commands:
//! - `loom memory note <text>` - Record a note
//! - `loom memory decision <text> [--context <ctx>]` - Record a decision
//! - `loom memory question <text>` - Record a question
//! - `loom memory query <search>` - Search memory entries
//! - `loom memory list [--stage <id>]` - List memory entries
//! - `loom memory show [--stage <id>] [--all]` - Show full memory journal
//! - `loom memory pending [--stage <id>]` - List entries without receipts
//! - `loom memory resolve <event-id> --outcome <outcome>` - Record a receipt

mod formatters;
mod handlers;

// Re-export all public command handlers
pub use handlers::change;
pub use handlers::decision;
pub use handlers::list;
pub use handlers::note;
pub use handlers::pending;
pub use handlers::query;
pub use handlers::question;
pub use handlers::resolve;
pub use handlers::show;

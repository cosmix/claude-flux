//! Fixed literal reminder boxes rendered inline in signal sections.
//!
//! Split out of `sections.rs` to keep its two large section builders under
//! their line ceilings; each function here just appends one fixed block.

/// The "📝 KNOWLEDGE UPDATES REQUIRED" box shown to knowledge-family stages.
pub(super) fn append_knowledge_updates_required_box(content: &mut String) {
    content.push_str("```text\n");
    content.push_str("┌────────────────────────────────────────────────────────────────────┐\n");
    content.push_str("│  📝 KNOWLEDGE UPDATES REQUIRED                                     │\n");
    content.push_str("│                                                                    │\n");
    content.push_str("│  As you work, UPDATE doc/loom/knowledge/:                          │\n");
    content.push_str("│  - Entry points: Key files you discover                            │\n");
    content.push_str("│  - Patterns: Architectural patterns you find                       │\n");
    content.push_str("│  - Conventions: Coding conventions you learn                       │\n");
    content.push_str("│  - Mistakes: Errors you make and how to avoid them                 │\n");
    content.push_str("│                                                                    │\n");
    content.push_str("│  Command: loom knowledge update <file> \"content\"                   │\n");
    content.push_str("└────────────────────────────────────────────────────────────────────┘\n");
    content.push_str("```\n\n");
}

/// The "NO MEMORY ENTRIES RECORDED" box shown when a stage's memory is empty.
pub(super) fn append_empty_memory_box(content: &mut String) {
    content.push_str("```\n");
    content.push_str("┌─────────────────────────────────────────────────────────────┐\n");
    content.push_str("│  ⚠️  NO MEMORY ENTRIES RECORDED — THIS IS A PROBLEM         │\n");
    content.push_str("│                                                             │\n");
    content.push_str("│  You should have been recording memories AS YOU WORKED:     │\n");
    content.push_str("│  - Every mistake/error you hit and how you fixed it         │\n");
    content.push_str("│  - Every non-obvious decision and WHY                       │\n");
    content.push_str("│  - Every surprise or gotcha in the code                     │\n");
    content.push_str("│                                                             │\n");
    content.push_str("│  BEFORE completing this stage, record what you learned:     │\n");
    content.push_str("│  BAD:  \"mistake: wrong path\"  (no context, useless)         │\n");
    content.push_str("│  GOOD: \"mistake: used loom/src/foo.rs in acceptance but     │\n");
    content.push_str("│    working_dir='loom' so path should be src/foo.rs.         │\n");
    content.push_str("│    Prevention: check working_dir before writing paths\"      │\n");
    content.push_str("│                                                             │\n");
    content.push_str("│  Empty memory = lost learning = repeated mistakes           │\n");
    content.push_str("└─────────────────────────────────────────────────────────────┘\n");
    content.push_str("```\n\n");
}

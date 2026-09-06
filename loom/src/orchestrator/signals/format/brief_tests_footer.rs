//! Footer tests: the "Pull more with" invocation the brief tells the reader
//! to run.
//!
//! Split out of `brief_tests.rs` (already near its own line budget) into a
//! sibling wired the same way `brief_tests_source.rs` is:
//! `#[path = "brief_tests_footer.rs"] mod footer_tests;`.

use super::super::format_knowledge_brief;
use super::{item, pack};

/// `None` is the checkout-scope caller's stage (`commands::hook::user_prompt`'s
/// `DeliveryTarget::for_checkout`), which names no stage on disk — the footer
/// must fall back to a bare `--query` rather than a `--stage` flag that would
/// fail with "Stage file not found".
#[test]
fn a_checkout_scope_brief_names_no_stage_in_its_footer() {
    let rendered = format_knowledge_brief(&pack(vec![item("chunk-1", None)], 0), None, "q");

    assert!(
        rendered.contains("loom knowledge context --query \"<question>\" --budget-tokens <n>"),
        "{rendered}"
    );
    assert!(!rendered.contains("--stage"), "{rendered}");
}

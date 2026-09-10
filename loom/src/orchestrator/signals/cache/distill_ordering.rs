//! The "record memories before you distill" doctrine block.
//!
//! Split out of `generate_knowledge_distill_stable_prefix` in the parent
//! module: the one-way-door explanation for why memory recording must finish
//! before distillation starts is a self-contained paragraph, not something
//! that function needs inline to read.

/// Append the CRITICAL ORDERING / one-way-door doctrine paragraph.
pub(super) fn append_memory_ordering_doctrine(content: &mut String) {
    content.push_str("**CRITICAL ORDERING — Record your OWN memories FIRST, then distill:**\n\n");
    content.push_str(
        "**⛔ MEMORY IS A ONE-WAY DOOR — recording to `loom memory` AFTER you distill is ZERO-VALUE WASTE.**\n",
    );
    content.push_str("This is the LAST stage of the plan: the moment distillation finishes, the plan completes and the\n");
    content.push_str("run state is archived to `<main>/.loom/memory/archive/<plan>-<timestamp>/` — deletion happens only\n");
    content.push_str("later, in `loom clean` / `loom init --clean`. After this stage, `loom review` and that archive are\n");
    content.push_str("the only readers of anything recorded here. Therefore:\n\n");
    content.push_str(
        "- Record ALL of your own findings to `loom memory` in step 1, BEFORE you begin step 4.\n",
    );
    content.push_str(
        "- Once you start distilling, STOP using `loom memory` entirely; anything discovered from then on goes\n",
    );
    content.push_str("  DIRECTLY into `loom knowledge update`, never back into memory.\n");
    content.push_str(
        "- At completion, do NOT run a \"record outstanding memories\" pass. There is nothing left to record.\n\n",
    );
}

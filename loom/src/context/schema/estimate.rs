/// Bytes of text approximated by one token.
///
/// Deliberately crude: see the module docs. Anything derived from this constant
/// must be named or documented as an *estimate*.
pub const BYTES_PER_TOKEN_ESTIMATE: usize = 4;

/// Estimated cost of the Knowledge Brief's header and footer around the items;
/// measured by `brief_tests::frame_cost_is_within_the_constant` (A3 owns that test).
pub const BRIEF_FRAME_TOKENS: usize = 128;

/// Estimate the token cost of a string.
///
/// This is an approximation, not a tokenizer. It is the single definition used
/// by the chunker, the ranker, and the packer so that a budget check and the
/// number it is checked against can never disagree.
pub fn estimate_tokens(text: &str) -> usize {
    text.len() / BYTES_PER_TOKEN_ESTIMATE
}

/// Hard ceiling on one item's quoted excerpt, in estimated tokens.
///
/// Independent of the retrieval budget: the budget decides *which* units are
/// worth paying for, this decides how much of one unit is worth quoting inline
/// rather than pointing at.
pub const EXCERPT_MAX_TOKENS: usize = 400;

/// Appended on its own line when an excerpt was cut short.
pub const EXCERPT_TRUNCATION_MARKER: &str = "[… truncated — open the pointer above for the rest]";

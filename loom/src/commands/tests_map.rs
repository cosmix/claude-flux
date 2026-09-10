use super::*;
use clap::Parser;

#[derive(Debug, Parser)]
struct MapHarness {
    #[command(flatten)]
    args: MapArgs,
}

#[test]
fn map_flags_parse_with_the_contract_defaults() {
    let parsed = MapHarness::try_parse_from(["map", "--impact", "target"]).unwrap();

    assert_eq!(parsed.args.depth, 3);
    assert_eq!(parsed.args.limit, 50);
    assert_eq!(parsed.args.min_confidence, 0.0);
    assert_eq!(parsed.args.kinds.len(), 6);
    assert!(!parsed.args.json);
}

#[test]
fn kinds_accept_the_stable_comma_separated_names() {
    let parsed =
        MapHarness::try_parse_from(["map", "--impact", "target", "--kinds", "calls,references"])
            .unwrap();

    assert_eq!(
        parsed.args.kinds,
        vec![SourceEdgeKind::Calls, SourceEdgeKind::References]
    );
}

#[test]
fn kinds_reject_an_unknown_name_and_list_every_valid_kind() {
    let error =
        MapHarness::try_parse_from(["map", "--impact", "target", "--kinds", "calls,unknown"])
            .expect_err("an unknown edge kind must be a clap error")
            .to_string();

    assert!(error.contains("unknown edge kind 'unknown'"));
    assert!(error.contains(EDGE_KIND_NAMES));
}

#[test]
fn map_without_a_view_flag_names_all_available_views() {
    let parsed = MapHarness::try_parse_from(["map"]).unwrap();

    let error = require_view(&parsed.args).unwrap_err().to_string();

    assert_eq!(
        error,
        "loom map needs a view flag: --outline <PATH>, --find-all <SYMBOL>, \
         --impact <SYMBOL_OR_PATH>, --callers <SYMBOL>, or --callees <SYMBOL>"
    );
}

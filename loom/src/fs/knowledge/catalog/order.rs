//! Issue ordering is what keeps `Catalog::revision` and every diagnostics diff stable.

use super::CatalogIssue;
use crate::fs::knowledge::types::INDEX_FILENAME;
use std::cmp::Ordering;
use std::path::Path;

pub(super) fn compare_issues(left: &CatalogIssue, right: &CatalogIssue) -> Ordering {
    issue_file(left)
        .cmp(issue_file(right))
        .then_with(|| issue_kind(left).cmp(&issue_kind(right)))
        .then_with(|| issue_payload(left).cmp(&issue_payload(right)))
}

fn issue_file(issue: &CatalogIssue) -> &Path {
    match issue {
        CatalogIssue::DuplicateHeading { file, .. }
        | CatalogIssue::GenericBlurb { file, .. }
        | CatalogIssue::BrokenLink { file, .. }
        | CatalogIssue::MissingSourceRef { file, .. }
        | CatalogIssue::EvidenceChanged { file, .. }
        | CatalogIssue::UnverifiableReference { file, .. }
        | CatalogIssue::OversizedSection { file, .. }
        | CatalogIssue::OversizedFile { file, .. } => file,
        CatalogIssue::OversizedIndex { .. } => Path::new(INDEX_FILENAME),
    }
}

fn issue_kind(issue: &CatalogIssue) -> u8 {
    match issue {
        CatalogIssue::DuplicateHeading { .. } => 0,
        CatalogIssue::GenericBlurb { .. } => 1,
        CatalogIssue::BrokenLink { .. } => 2,
        CatalogIssue::MissingSourceRef { .. } => 3,
        CatalogIssue::EvidenceChanged { .. } => 4,
        CatalogIssue::UnverifiableReference { .. } => 5,
        CatalogIssue::OversizedSection { .. } => 6,
        CatalogIssue::OversizedFile { .. } => 7,
        CatalogIssue::OversizedIndex { .. } => 8,
    }
}

fn issue_payload(issue: &CatalogIssue) -> String {
    match issue {
        CatalogIssue::DuplicateHeading {
            heading,
            occurrences,
            ..
        } => format!("{heading}:{occurrences}"),
        CatalogIssue::GenericBlurb { blurb, .. } => blurb.clone(),
        CatalogIssue::BrokenLink { target, .. } => target.clone(),
        CatalogIssue::MissingSourceRef { source_path, .. } => source_path.clone(),
        CatalogIssue::EvidenceChanged {
            source_path,
            verified,
            ..
        } => format!("{source_path}:{verified}"),
        CatalogIssue::UnverifiableReference {
            source_path, kind, ..
        } => format!("{source_path}:{kind}"),
        CatalogIssue::OversizedSection { heading, lines, .. } => format!("{heading}:{lines}"),
        CatalogIssue::OversizedFile { lines, .. } => lines.to_string(),
        CatalogIssue::OversizedIndex { bytes } => bytes.to_string(),
    }
}

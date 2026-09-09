use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use loom::context::config::RetrievalConfig;
use loom::context::fuse::fuse;
use loom::context::pack::{pack, PackRequest};
use loom::context::rank::{rank, RankQuery};
use loom::context::schema::{Channel, Freshness, RequiredRepresentation, BRIEF_FRAME_TOKENS};
use loom::context::store::{canonical_json, ContextStore};
use loom::fs::knowledge::catalog::{build, CatalogIssue};
use tempfile::TempDir;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/knowledge")
        .join(name)
}

fn copy_tree(source: &Path, destination: &Path) -> io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&source_path, &destination_path)?;
        } else {
            fs::copy(source_path, destination_path)?;
        }
    }
    Ok(())
}

fn markdown_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_markdown_files(root, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_markdown_files(directory: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_markdown_files(&path, files)?;
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("md") {
            files.push(path);
        }
    }
    Ok(())
}

fn pack_request(query: &str, budget_tokens: usize) -> PackRequest {
    PackRequest {
        query: query.to_string(),
        scope: vec![Channel::Knowledge],
        budget_tokens,
        structural_freshness: Freshness::default(),
        semantic_freshness: Freshness::default(),
        dropped_terms: Vec::new(),
        surviving_terms: Vec::new(),
        required_representation: RequiredRepresentation::default(),
        degraded: None,
    }
}

#[test]
fn cold_and_incremental_catalog_builds_are_byte_identical() -> anyhow::Result<()> {
    let source = fixture("hierarchical");
    let first = build(&source)?;
    let second = build(&source)?;
    assert_eq!(first.revision, second.revision);
    assert_eq!(canonical_json(&first)?, canonical_json(&second)?);

    let store_directory = TempDir::new()?;
    let copied_directory = TempDir::new()?;
    let copied_root = copied_directory.path().join("knowledge");
    copy_tree(&source, &copied_root)?;
    let store = ContextStore::with_root(store_directory.path());

    let first_refresh = loom::context::refresh::refresh(&store, &copied_root, true)?;
    assert!(first_refresh.rebuilt);
    let first_catalog = canonical_json(&store.load_catalog()?.expect("catalog after rebuild"))?;

    let second_refresh = loom::context::refresh::refresh(&store, &copied_root, true)?;
    assert!(!second_refresh.rebuilt);
    let second_catalog = canonical_json(&store.load_catalog()?.expect("catalog after no-op"))?;
    assert_eq!(first_catalog, second_catalog);
    Ok(())
}

#[test]
fn flat_tree_builds_and_is_never_modified() -> anyhow::Result<()> {
    let root = fixture("flat");
    let before: Vec<_> = markdown_files(&root)?
        .into_iter()
        .map(|path| {
            let bytes = fs::read(&path)?;
            let modified = fs::metadata(&path)?.modified()?;
            Ok::<_, io::Error>((path, bytes, modified))
        })
        .collect::<Result<_, _>>()?;

    let catalog = build(&root)?;
    assert!(!catalog.chunks.is_empty());

    for (path, bytes, modified) in before {
        assert_eq!(
            fs::read(&path)?,
            bytes,
            "fixture bytes changed: {}",
            path.display()
        );
        assert_eq!(
            fs::metadata(&path)?.modified()?,
            modified,
            "fixture mtime changed: {}",
            path.display()
        );
    }
    Ok(())
}

#[test]
fn hierarchical_tree_reports_expected_issues() -> anyhow::Result<()> {
    let root = fixture("hierarchical");
    let before: Vec<_> = markdown_files(&root)?
        .into_iter()
        .map(|path| Ok::<_, io::Error>((path.clone(), fs::read(path)?)))
        .collect::<Result<_, _>>()?;

    let catalog = build(&root)?;
    assert!(catalog
        .issues
        .iter()
        .any(|issue| matches!(issue, CatalogIssue::DuplicateHeading { occurrences: 2, .. })));
    assert!(catalog
        .issues
        .iter()
        .any(|issue| matches!(issue, CatalogIssue::GenericBlurb { .. })));
    assert!(catalog.issues.iter().any(|issue| matches!(
        issue,
        CatalogIssue::BrokenLink { target, .. }
            if target == "architecture/does-not-exist.md"
    )));

    for (path, bytes) in before {
        assert_eq!(
            fs::read(&path)?,
            bytes,
            "fixture bytes changed: {}",
            path.display()
        );
    }
    Ok(())
}

#[test]
fn fenced_code_headings_do_not_split_sections() -> anyhow::Result<()> {
    let catalog = build(&fixture("hierarchical"))?;
    let fenced_chunks: Vec<_> = catalog
        .chunks
        .iter()
        .filter(|chunk| chunk.file.ends_with("fenced-code.md"))
        .collect();

    assert_eq!(fenced_chunks.len(), 2);
    assert!(fenced_chunks
        .iter()
        .all(|chunk| chunk.heading != "Hidden backtick heading"));
    assert!(fenced_chunks
        .iter()
        .all(|chunk| chunk.heading != "Hidden tilde heading"));
    Ok(())
}

#[test]
fn pack_at_200_tokens_never_exceeds_200() -> anyhow::Result<()> {
    let catalog = build(&fixture("hierarchical"))?;
    let query = RankQuery {
        text: "locking".to_string(),
        ..RankQuery::default()
    };
    let config = RetrievalConfig::default();
    let ranked = rank(&query, &catalog.chunks, Channel::Knowledge, &config);
    let fused = fuse(&[ranked]);

    let packed = pack(&pack_request("locking", 200), &fused, &catalog.chunks, None);
    assert!(packed.estimated_tokens <= 200);
    assert!(packed.within_budget());

    let empty = pack(&pack_request("locking", 0), &fused, &catalog.chunks, None);
    assert!(empty.items.is_empty());
    Ok(())
}

#[test]
fn every_pack_reports_omissions_and_coverage() -> anyhow::Result<()> {
    let catalog = build(&fixture("hierarchical"))?;
    let query = RankQuery {
        text: "locking".to_string(),
        ..RankQuery::default()
    };
    let config = RetrievalConfig::default();
    let ranked = rank(&query, &catalog.chunks, Channel::Knowledge, &config);
    let fused = fuse(&[ranked]);

    for budget in [0, 50, 200, 100_000] {
        let packed = pack(
            &pack_request("locking", budget),
            &fused,
            &catalog.chunks,
            None,
        );
        assert_eq!(
            packed.omitted.coverage.included + packed.omitted.omitted,
            packed.omitted.coverage.candidates
        );
        assert_eq!(packed.omitted.coverage.included, packed.items.len());
        assert_eq!(
            packed.omitted.coverage.included_tokens + BRIEF_FRAME_TOKENS,
            packed.estimated_tokens,
            "the pack estimate is the brief frame plus the rendered cost of what it carries"
        );
    }
    Ok(())
}

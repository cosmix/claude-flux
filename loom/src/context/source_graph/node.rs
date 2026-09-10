//! Node types for the derived source graph: [`SourceNode`], its
//! [`SourceNodeKind`], [`Span`], [`FileCoverage`], and [`NodeLanguage`].

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::language::DetectedLanguage;

/// What kind of program element a [`SourceNode`] denotes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceNodeKind {
    /// Whole-file node. Always present, even for unsupported languages.
    File,
    /// A free function, method, or closure bound to a name.
    Function,
    /// A struct, class, enum, or record type definition.
    Type,
    /// A trait, interface, or protocol.
    Interface,
    /// A module, namespace, or package declaration.
    Module,
    /// A constant, static, or module-level variable binding.
    Constant,
    /// An `impl` block, `extends` clause, or equivalent grouping construct.
    Implementation,
}

impl SourceNodeKind {
    /// Stable lowercase name, used in ids and CLI output.
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceNodeKind::File => "file",
            SourceNodeKind::Function => "function",
            SourceNodeKind::Type => "type",
            SourceNodeKind::Interface => "interface",
            SourceNodeKind::Module => "module",
            SourceNodeKind::Constant => "constant",
            SourceNodeKind::Implementation => "implementation",
        }
    }
}

impl std::fmt::Display for SourceNodeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A half-open byte range `[start, end)` into the file's exact bytes.
///
/// Byte offsets rather than line numbers because they survive any encoding and
/// are what tree-sitter reports natively; `line_start`/`line_end` are carried
/// alongside for human-facing output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Span {
    /// Inclusive start byte offset.
    pub start_byte: usize,
    /// Exclusive end byte offset.
    pub end_byte: usize,
    /// 1-indexed inclusive start line.
    pub line_start: usize,
    /// 1-indexed inclusive end line.
    pub line_end: usize,
}

/// How completely a file was extracted.
///
/// A file never disappears from the graph: an oversized or unparseable file
/// keeps its file-level node and records why its symbols are missing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum FileCoverage {
    /// The file was deleted from this overlay.
    Deleted,
    /// The whole file parsed and every query match was walked.
    Full,
    /// The file was parsed by a real grammar but the extraction is partial.
    Partial {
        /// Human-readable cause, e.g. "12 query matches had no named capture".
        detail: String,
    },
    /// No grammar for this language — only a file-level lexical node exists.
    LexicalOnly {
        /// Why the language could not be parsed.
        detail: String,
    },
    /// The file exceeded
    /// [`crate::context::source_graph::MAX_EXTRACTED_FILE_BYTES`] and was not
    /// parsed. File-level metadata is retained.
    Oversized {
        /// Actual size in bytes.
        bytes: usize,
        /// The cap that was exceeded.
        limit: usize,
    },
    /// The grammar reported a syntax error. No symbol nodes are emitted.
    ParseError {
        /// Byte span of the first error node reported by the grammar.
        span: Span,
        /// Human-readable description of the failure.
        detail: String,
    },
}

impl FileCoverage {
    /// Stable lowercase status name, used in CLI output and fixture JSON.
    pub fn status(&self) -> &'static str {
        match self {
            FileCoverage::Deleted => "deleted",
            FileCoverage::Full => "full",
            FileCoverage::Partial { .. } => "partial",
            FileCoverage::LexicalOnly { .. } => "lexical-only",
            FileCoverage::Oversized { .. } => "oversized",
            FileCoverage::ParseError { .. } => "parse-error",
        }
    }

    /// True when symbol-level nodes are expected to be present.
    pub fn has_symbols(&self) -> bool {
        matches!(self, FileCoverage::Full | FileCoverage::Partial { .. })
    }
}

/// One node of the derived source graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceNode {
    /// Stable id: the bare relative path for a file node, and
    /// `<relative-path>#<scope-path>` for a symbol node (see
    /// [`crate::context::source_graph::node_id`]).
    pub id: String,
    pub kind: SourceNodeKind,
    /// Path relative to the project root, always forward-slash separated.
    pub path: PathBuf,
    /// Enclosing scope segments, outermost first. Empty at file level.
    #[serde(default)]
    pub scope: Vec<String>,
    pub span: Span,
    /// Declaration text as written, without the body. Empty for a file node.
    #[serde(default)]
    pub signature: String,
    /// `sha256:<hex>` over the node's exact source bytes.
    pub body_hash: String,
    /// Language the extractor that produced this node handles.
    pub language: NodeLanguage,
    /// Identity of the extractor build, so a cached node can be invalidated
    /// when the grammar or the query changes.
    pub parser_version: String,
    /// Coverage of the file this node came from.
    pub coverage: FileCoverage,
}

/// Language tag on a [`SourceNode`].
///
/// Mirrors [`DetectedLanguage`] plus an `Other` arm, because a file-level
/// lexical node exists for every tracked file regardless of language and
/// [`DetectedLanguage`] deliberately enumerates only the languages loom
/// supports elsewhere.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NodeLanguage {
    Rust,
    TypeScript,
    Python,
    Go,
    /// Any other language: file-level lexical coverage only. Carries the
    /// extension so output can still say what it was.
    Other(String),
}

impl NodeLanguage {
    /// Stable lowercase name used in ids and CLI output.
    pub fn as_str(&self) -> &str {
        match self {
            NodeLanguage::Rust => "rust",
            NodeLanguage::TypeScript => "typescript",
            NodeLanguage::Python => "python",
            NodeLanguage::Go => "go",
            NodeLanguage::Other(ext) => ext,
        }
    }
}

impl From<DetectedLanguage> for NodeLanguage {
    fn from(language: DetectedLanguage) -> Self {
        match language {
            DetectedLanguage::Rust => NodeLanguage::Rust,
            DetectedLanguage::TypeScript => NodeLanguage::TypeScript,
            DetectedLanguage::Python => NodeLanguage::Python,
            DetectedLanguage::Go => NodeLanguage::Go,
        }
    }
}

impl std::fmt::Display for NodeLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

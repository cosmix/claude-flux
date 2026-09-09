use std::collections::BTreeSet;
use std::fs::File;
use std::io::Read;
use std::path::Path;

const FILE_MARKERS: &[(&str, &[&str])] = &[
    ("rust", &["Cargo.toml"]),
    ("golang", &["go.mod", "go.sum"]),
    (
        "python",
        &[
            "pyproject.toml",
            "setup.py",
            "requirements.txt",
            "Pipfile",
            "poetry.lock",
        ],
    ),
    ("typescript", &["tsconfig.json"]),
    (
        "react",
        &[
            "next.config.js",
            "next.config.ts",
            "next.config.mjs",
            "next.config.cjs",
            "remix.config.js",
        ],
    ),
    (
        "docker",
        &[
            "Dockerfile",
            "docker-compose.yml",
            "docker-compose.yaml",
            "compose.yaml",
            "compose.yml",
        ],
    ),
    ("kustomize", &["kustomization.yaml", "kustomization.yml"]),
    (
        "kubernetes",
        &[
            "kustomization.yaml",
            "kustomization.yml",
            "Chart.yaml",
            "helmfile.yaml",
            "helmfile.yml",
            "skaffold.yaml",
        ],
    ),
    (
        "terraform",
        &[
            "main.tf",
            "terraform.tf",
            ".terraform.lock.hcl",
            "versions.tf",
        ],
    ),
    (
        "ci-cd",
        &[
            ".gitlab-ci.yml",
            ".circleci/config.yml",
            "azure-pipelines.yml",
            "Jenkinsfile",
        ],
    ),
    ("fluxcd", &["flux-system.yaml"]),
    ("prometheus", &["prometheus.yml", "prometheus.yaml"]),
    ("grafana", &["grafana.ini"]),
];

pub(super) fn detect(dir: &Path) -> BTreeSet<String> {
    let mut types = BTreeSet::new();
    for (kind, files) in FILE_MARKERS {
        if files.iter().any(|name| regular_file(&dir.join(name))) {
            types.insert((*kind).to_string());
        }
    }
    for (kind, names) in [
        ("ci-cd", &[".github/workflows"][..]),
        ("argocd", &["argocd", ".argocd"][..]),
        ("fluxcd", &["flux", ".flux"][..]),
        ("grafana", &["grafana"][..]),
    ] {
        if names.iter().any(|name| plain_directory(&dir.join(name))) {
            types.insert(kind.to_string());
        }
    }
    detect_dependencies(dir, &mut types);
    types
}

fn detect_dependencies(dir: &Path, types: &mut BTreeSet<String>) {
    let path = dir.join("package.json");
    if !regular_file(&path) {
        return;
    }
    let Some(package) = read_json(&path) else {
        return;
    };
    for section in ["dependencies", "devDependencies", "peerDependencies"] {
        let Some(deps) = package.get(section).and_then(serde_json::Value::as_object) else {
            continue;
        };
        if deps.contains_key("typescript") {
            types.insert("typescript".into());
        }
        if [
            "react",
            "react-dom",
            "next",
            "remix",
            "@remix-run/react",
            "@remix-run/node",
        ]
        .iter()
        .any(|name| deps.contains_key(*name))
        {
            types.insert("react".into());
        }
    }
}

fn read_json(path: &Path) -> Option<serde_json::Value> {
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take(256 * 1024 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 256 * 1024 {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

pub(super) fn regular_file(path: &Path) -> bool {
    path.symlink_metadata()
        .is_ok_and(|meta| meta.file_type().is_file())
}

pub(super) fn plain_directory(path: &Path) -> bool {
    path.symlink_metadata()
        .is_ok_and(|meta| meta.file_type().is_dir())
}

pub(super) fn package_boundary(path: &Path) -> bool {
    [
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        "go.mod",
        "tsconfig.json",
    ]
    .iter()
    .any(|name| regular_file(&path.join(name)))
}

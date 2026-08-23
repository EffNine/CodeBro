//! Deterministic project identity inference for the init pipeline.
//!
//! [`infer_identity`] reads the workspace surface (manifests, README, git
//! remote) and derives what can be known without any model in the loop:
//! description, repository URL, build system, package manager, testing
//! framework, frameworks, and important files.
//!
//! Inference is conservative: a field stays `None` when no deterministic
//! signal exists. The init pipeline applies inferred values only to fields
//! the human/agent has not already authored, so re-running init never
//! clobbers curated identity data.

use std::collections::BTreeSet;
use std::path::Path;

/// What could be deterministically inferred from the workspace surface.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InferredIdentity {
    pub description: Option<String>,
    pub repository_url: Option<String>,
    pub build_system: Option<String>,
    pub package_manager: Option<String>,
    pub testing_framework: Option<String>,
    /// Sorted, de-duplicated framework names (not raw dependencies).
    pub frameworks: Vec<String>,
    /// Existing files worth surfacing as important, sorted.
    pub important_files: Vec<String>,
}

/// Known dependency → framework-name mappings per ecosystem. Deliberately
/// small: only entries that name an actual framework or platform-level
/// library, not ordinary dependencies.
const RUST_FRAMEWORKS: &[(&str, &str)] = &[
    ("tokio", "tokio"),
    ("async-std", "async-std"),
    ("actix-web", "actix-web"),
    ("axum", "axum"),
    ("rocket", "rocket"),
    ("warp", "warp"),
    ("rmcp", "rmcp"),
    ("tauri", "tauri"),
    ("sqlx", "sqlx"),
    ("diesel", "diesel"),
    ("tonic", "tonic"),
    ("clap", "clap"),
];

const GO_FRAMEWORKS: &[(&str, &str)] = &[
    ("github.com/gin-gonic/gin", "gin"),
    ("github.com/labstack/echo", "echo"),
    ("github.com/spf13/cobra", "cobra"),
    ("github.com/go-chi/chi", "chi"),
];

const JS_FRAMEWORKS: &[(&str, &str)] = &[
    ("react", "react"),
    ("vue", "vue"),
    ("svelte", "svelte"),
    ("next", "next"),
    ("nuxt", "nuxt"),
    ("express", "express"),
    ("fastify", "fastify"),
    ("vite", "vite"),
];

/// Infer identity facts from the workspace root. Never touches the
/// network; `git remote` is the only subprocess and its failure is fine.
pub fn infer_identity(root: &Path) -> InferredIdentity {
    let mut out = InferredIdentity::default();
    let mut important: BTreeSet<String> = BTreeSet::new();

    if let Some(text) = read_non_empty(&root.join("Cargo.toml")) {
        out.build_system = Some("cargo".to_string());
        out.package_manager = Some("cargo".to_string());
        out.testing_framework = Some("cargo test".to_string());
        important.insert("Cargo.toml".to_string());
        apply_cargo(&text, &mut out);
    } else if read_non_empty(&root.join("go.mod")).is_some() {
        out.build_system = Some("go".to_string());
        out.package_manager = Some("go".to_string());
        out.testing_framework = Some("go test".to_string());
        important.insert("go.mod".to_string());
        if let Some(text) = read_non_empty(&root.join("go.mod")) {
            collect_go_frameworks(&text, &mut out);
        }
    } else if read_non_empty(&root.join("package.json")).is_some() {
        important.insert("package.json".to_string());
        let pm = if root.join("pnpm-lock.yaml").exists() {
            "pnpm"
        } else if root.join("yarn.lock").exists() {
            "yarn"
        } else {
            "npm"
        };
        out.build_system = Some(pm.to_string());
        out.package_manager = Some(pm.to_string());
        out.testing_framework = Some(format!("{pm} test"));
        if let Some(text) = read_non_empty(&root.join("package.json")) {
            collect_js_frameworks(&text, &mut out);
        }
    }

    for candidate in [
        "README.md",
        "readme.md",
        "README",
        "src/main.rs",
        "src/lib.rs",
        "main.go",
        "index.js",
        "index.ts",
    ] {
        if root.join(candidate).is_file() {
            important.insert(candidate.to_string());
        }
    }

    if out.description.is_none() {
        out.description = first_readme_paragraph(root);
    }

    if out.repository_url.is_none() {
        out.repository_url = git_origin_url(root);
    }

    out.important_files = important.into_iter().collect();
    out
}

fn apply_cargo(text: &str, out: &mut InferredIdentity) {
    let Ok(value) = text.parse::<toml::Value>() else {
        return;
    };
    let package = value.get("package");
    if let Some(desc) = package.and_then(|p| p.get("description")).and_then(|d| d.as_str()) {
        out.description = normalize_description(desc);
    }
    if let Some(url) = package.and_then(|p| p.get("repository")).and_then(|r| r.as_str()) {
        out.repository_url = Some(url.to_string());
    }

    // Framework detection over declared dependencies only ([dependencies],
    // plus target-specific tables), never dev-dependencies or build deps —
    // a dev-only tool does not make a framework part of the product.
    let mut found: BTreeSet<String> = BTreeSet::new();
    if let Some(deps) = value.get("dependencies").and_then(|d| d.as_table()) {
        for (name, _) in deps {
            if let Some((_, fw)) = RUST_FRAMEWORKS.iter().find(|(k, _)| k == name) {
                found.insert(fw.to_string());
            }
        }
    }
    if let Some(targets) = value.get("target").and_then(|t| t.as_table()) {
        for target in targets.values() {
            if let Some(deps) = target.get("dependencies").and_then(|d| d.as_table()) {
                for (name, _) in deps {
                    if let Some((_, fw)) = RUST_FRAMEWORKS.iter().find(|(k, _)| k == name) {
                        found.insert(fw.to_string());
                    }
                }
            }
        }
    }
    out.frameworks = found.into_iter().collect();
}

fn collect_go_frameworks(go_mod_text: &str, out: &mut InferredIdentity) {
    let mut found: BTreeSet<String> = BTreeSet::new();
    for line in go_mod_text.lines() {
        let line = line.trim();
        for (dep, fw) in GO_FRAMEWORKS {
            if line.starts_with(dep)
                || line.starts_with(&format!("\"{dep}"))
            {
                found.insert(fw.to_string());
            }
        }
    }
    out.frameworks = found.into_iter().collect();
}

fn collect_js_frameworks(package_json_text: &str, out: &mut InferredIdentity) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(package_json_text) else {
        return;
    };
    let mut found: BTreeSet<String> = BTreeSet::new();
    for section in ["dependencies", "peerDependencies"] {
        if let Some(deps) = value.get(section).and_then(|d| d.as_object()) {
            for name in deps.keys() {
                let base = name.split('/').last().unwrap_or(name);
                if let Some((_, fw)) = JS_FRAMEWORKS.iter().find(|(k, _)| *k == base || *k == name) {
                    found.insert(fw.to_string());
                }
            }
        }
    }
    if let Some(desc) = value.get("description").and_then(|d| d.as_str()) {
        if out.description.is_none() {
            out.description = normalize_description(desc);
        }
    }
    out.frameworks = found.into_iter().collect();
}

/// First meaningful paragraph of README.md: skip HTML comments, badge
/// lines, headings and blank runs; take up to two sentences, hard-capped.
fn first_readme_paragraph(root: &Path) -> Option<String> {
    for name in ["README.md", "readme.md", "README"] {
        if let Some(text) = read_non_empty(&root.join(name)) {
            for block in text.split("\n\n") {
                let mut lines: Vec<&str> = Vec::new();
                for line in block.lines() {
                    let t = line.trim();
                    if t.is_empty()
                        || t.starts_with('#')
                        || t.starts_with("<!--")
                        || t.contains("img.shields.io")
                        || t.starts_with("[!")
                        || t.starts_with('|')
                    {
                        continue;
                    }
                    lines.push(t);
                }
                if lines.is_empty() {
                    continue;
                }
                let joined = lines.join(" ");
                if joined.len() < 16 {
                    continue;
                }
                return normalize_description(&joined);
            }
        }
    }
    None
}

/// Trim to a sane single-line description: at most 2 sentences, at most
/// 240 chars, always ending on a clean word boundary with a period.
fn normalize_description(s: &str) -> Option<String> {
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    let mut sentences: Vec<&str> = vec![];
    for sent in collapsed.split_inclusive(|c| c == '.' || c == '!' || c == '?') {
        sentences.push(sent.trim());
        if sentences.len() == 2 {
            break;
        }
    }
    let mut desc = sentences.join(" ");
    if !desc.ends_with('.') && !desc.ends_with('!') && !desc.ends_with('?') {
        desc.push('.');
    }
    const MAX: usize = 240;
    if desc.len() > MAX {
        let cut = desc[..MAX]
            .rfind(char::is_whitespace)
            .unwrap_or(MAX);
        desc = format!("{}…", desc[..cut].trim_end());
    }
    (!desc.is_empty()).then_some(desc)
}

fn git_origin_url(root: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!url.is_empty()).then_some(url)
}

fn read_non_empty(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_manifest_yields_description_build_system_and_frameworks() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"probe\"\ndescription = \"A probe for testing inference. Extra sentence here.\"\nrepository = \"https://github.com/example/probe\"\n\n[dependencies]\ntokio = \"1\"\nserde = \"1\"\nclap = \"4\"\n",
        )
        .unwrap();
        let inferred = infer_identity(dir.path());
        assert_eq!(
            inferred.description.as_deref(),
            Some("A probe for testing inference. Extra sentence here.")
        );
        assert_eq!(inferred.repository_url.as_deref(), Some("https://github.com/example/probe"));
        assert_eq!(inferred.build_system.as_deref(), Some("cargo"));
        assert_eq!(inferred.package_manager.as_deref(), Some("cargo"));
        assert_eq!(inferred.testing_framework.as_deref(), Some("cargo test"));
        // serde/clap are ordinary deps; only platform-level crates count.
        assert_eq!(inferred.frameworks, vec!["clap".to_string(), "tokio".to_string()]);
        assert!(inferred.important_files.contains(&"Cargo.toml".to_string()));
    }

    #[test]
    fn readme_fills_description_when_manifest_is_silent() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"probe\"\n").unwrap();
        std::fs::write(
            dir.path().join("README.md"),
            "# probe\n\n[![badge](img.shields.io/x)](x)\n\nDoes something useful for people. Second line adds detail.\n",
        )
        .unwrap();
        let inferred = infer_identity(dir.path());
        let desc = inferred.description.expect("README paragraph used");
        assert!(desc.starts_with("Does something useful"), "got: {desc}");
        assert!(!desc.contains("shields"));
    }

    #[test]
    fn go_mod_yields_go_toolchain_and_frameworks() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("go.mod"),
            "module github.com/example/app\n\ngo 1.21\n\nrequire (\n\tgithub.com/gin-gonic/gin v1.9.0\n)\n",
        )
        .unwrap();
        let inferred = infer_identity(dir.path());
        assert_eq!(inferred.build_system.as_deref(), Some("go"));
        assert_eq!(inferred.testing_framework.as_deref(), Some("go test"));
        assert_eq!(inferred.frameworks, vec!["gin".to_string()]);
    }

    #[test]
    fn package_json_detects_pnpm_and_frameworks() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            "{\"name\":\"web\",\"dependencies\":{\"react\":\"^18\"}}",
        )
        .unwrap();
        std::fs::write(dir.path().join("pnpm-lock.yaml"), "").unwrap();
        let inferred = infer_identity(dir.path());
        assert_eq!(inferred.package_manager.as_deref(), Some("pnpm"));
        assert_eq!(inferred.frameworks, vec!["react".to_string()]);
    }

    #[test]
    fn empty_workspace_infers_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let inferred = infer_identity(dir.path());
        assert_eq!(inferred, InferredIdentity::default());
    }
}

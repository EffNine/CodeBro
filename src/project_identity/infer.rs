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
    /// Decisions mined from Architecture Decision Records — human-authored
    /// content parsed verbatim, not guessed.
    pub decisions: Vec<MinedDecision>,
    /// Conventions mined from an explicit "Conventions" section in
    /// AGENTS.md / CLAUDE.md.
    pub conventions: Vec<String>,
    /// Release milestones mined from CHANGELOG.md headings.
    pub milestones: Vec<String>,
}

/// A decision extracted from an ADR file.
#[derive(Debug, Clone, PartialEq)]
pub struct MinedDecision {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    /// "accepted" | "proposed" | "deprecated" | "superseded"
    pub status: &'static str,
    /// Source file relative to the workspace root.
    pub source_file: String,
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

    // Mine human-authored intent from documentation. These are parsed
    // verbatim from files the project's own authors wrote — not guessed.
    out.decisions = mine_adr_decisions(root);
    out.conventions = mine_conventions(root);
    out.milestones = mine_changelog_milestones(root);

    out
}

const MAX_MINED_DECISIONS: usize = 20;
const MAX_MINED_CONVENTIONS: usize = 15;
const MAX_MINED_MILESTONES: usize = 8;

/// Parse Architecture Decision Records (`docs/ADR/*.md` and variants).
///
/// Recognises both the bold-key header style and the section style:
///
/// ```text
/// **Status:** Accepted          |   ## Status
///                               |   Accepted
/// ```
fn mine_adr_decisions(root: &Path) -> Vec<MinedDecision> {
    let mut out = Vec::new();
    for dir in ["docs/ADR", "docs/adr", "ADR", "adr"] {
        let dir_path = root.join(dir);
        let Ok(entries) = std::fs::read_dir(&dir_path) else {
            continue;
        };
        let mut files: Vec<std::path::PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md"))
            .collect();
        files.sort();
        for file in files {
            if out.len() >= MAX_MINED_DECISIONS {
                return out;
            }
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            let rel = file
                .strip_prefix(root)
                .unwrap_or(&file)
                .to_string_lossy()
                .to_string();
            let stem = file
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("decision");
            let Some((title, status, description)) = parse_adr(&text) else {
                continue;
            };
            out.push(MinedDecision {
                id: slug(stem),
                title,
                description,
                status,
                source_file: rel,
            });
        }
    }
    out
}

/// Extract (title, status, description) from ADR markdown content.
fn parse_adr(text: &str) -> Option<(String, &'static str, Option<String>)> {
    let mut title = String::new();
    for line in text.lines() {
        if let Some(h) = line.strip_prefix("# ") {
            title = h.trim().to_string();
            break;
        }
    }
    if title.is_empty() {
        return None;
    }
    // Strip a leading "ADR-012:" numbering prefix from the display title.
    let title = match title.split_once(": ") {
        Some((prefix, rest)) if prefix.starts_with("ADR") => rest.trim().to_string(),
        _ => title,
    };

    let mut status: &'static str = "proposed";
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("**Status:**") {
            status = map_status(rest);
            break;
        }
        if trimmed.eq_ignore_ascii_case("## status") || trimmed.eq_ignore_ascii_case("# status") {
            for next in lines.by_ref() {
                let t = next.trim();
                if t.is_empty() {
                    continue;
                }
                status = map_status(t);
                break;
            }
            break;
        }
    }

    // Description: first paragraph of a Context section, else None.
    let mut description = None;
    let mut in_context = false;
    let mut para: Vec<&str> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        let is_heading = trimmed.starts_with('#');
        if is_heading {
            if in_context && !para.is_empty() {
                break;
            }
            in_context = trimmed
                .trim_start_matches('#')
                .trim()
                .eq_ignore_ascii_case("context");
            continue;
        }
        if in_context {
            if trimmed.is_empty() {
                if !para.is_empty() {
                    break;
                }
            } else if !trimmed.starts_with("**") && !trimmed.starts_with("---") {
                para.push(trimmed);
            }
        }
    }
    if !para.is_empty() {
        description = normalize_description(&para.join(" "));
    }

    Some((title, status, description))
}

fn map_status(raw: &str) -> &'static str {
    let lowered = raw.trim().trim_matches('*').to_lowercase();
    if lowered.contains("accept") {
        "accepted"
    } else if lowered.contains("deprecat") {
        "deprecated"
    } else if lowered.contains("superseded") || lowered.contains("superceded") {
        "superseded"
    } else {
        "proposed"
    }
}

/// Collect bullet list items under an explicit `Conventions` heading from
/// AGENTS.md / CLAUDE.md at the workspace root.
fn mine_conventions(root: &Path) -> Vec<String> {
    for name in ["AGENTS.md", "CLAUDE.md"] {
        let Ok(text) = std::fs::read_to_string(root.join(name)) else {
            continue;
        };
        let mut conventions: Vec<String> = Vec::new();
        let mut in_section = false;
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('#') {
                if in_section && !conventions.is_empty() {
                    break;
                }
                in_section = trimmed
                    .trim_start_matches('#')
                    .trim()
                    .to_lowercase()
                    .starts_with("convention");
                continue;
            }
            if in_section {
                if let Some(item) = trimmed.strip_prefix("- ") {
                    let item = item.trim();
                    if !item.is_empty() && conventions.len() < MAX_MINED_CONVENTIONS {
                        conventions.push(item.to_string());
                    }
                }
            }
        }
        if !conventions.is_empty() {
            return conventions;
        }
    }
    Vec::new()
}

/// Release milestones from CHANGELOG.md top-level headings:
/// `## [0.7.0-mcp-rc2] - 2026-08-17` → `v0.7.0-mcp-rc2`.
fn mine_changelog_milestones(root: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(root.join("CHANGELOG.md")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("## ") {
            continue;
        }
        let heading = trimmed.trim_start_matches('#').trim();
        let version = heading
            .trim_start_matches('[')
            .split(']')
            .next()
            .unwrap_or(heading)
            .trim();
        if version.is_empty() || version.eq_ignore_ascii_case("unreleased") {
            continue;
        }
        let milestone = if version.starts_with('v') {
            version.to_string()
        } else {
            format!("v{version}")
        };
        if !out.contains(&milestone) {
            out.push(milestone);
        }
        if out.len() >= MAX_MINED_MILESTONES {
            break;
        }
    }
    out
}

/// Deterministic slug shared by the mining path: lowercase,
/// non-alphanumeric runs collapsed to `-`, capped at 80 chars.
fn slug(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= 80 {
            break;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        out.push_str("item");
    }
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
                    // Strip common markdown noise from the leading edge.
                    let cleaned = t
                        .trim_start_matches('>')
                        .trim_start_matches(['*', '_'])
                        .trim_start();
                    lines.push(cleaned);
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

#[cfg(test)]
mod mining_tests {
    use super::*;

    #[test]
    fn mines_adrs_conventions_and_milestones() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("docs/ADR")).unwrap();
        std::fs::write(
            dir.path().join("docs/ADR/ADR-001-use-rust.md"),
            "# ADR-001: Use Rust\n\n**Status:** Accepted\n\n## Context\nWe need memory safety and a single binary deliverable.\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("docs/ADR/ADR-002-try-go.md"),
            "# ADR-002: Try Go first\n\n## Status\nProposed\n\n## Context\nAlternative considered early on.\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("AGENTS.md"),
            "# Guide\n\nSome intro.\n\n### Conventions\n\n- Tests use tempdir.\n- Never commit secrets.\n\n## Other\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("CHANGELOG.md"),
            "# Changelog\n\n## [Unreleased]\n- x\n\n## [1.2.0] - 2026-01-01\n- y\n\n## [1.0.0] - 2025-12-01\n- z\n",
        )
        .unwrap();

        let inferred = infer_identity(dir.path());
        assert_eq!(inferred.decisions.len(), 2);
        let d1 = &inferred.decisions[0];
        assert_eq!(d1.id, "adr-001-use-rust");
        assert_eq!(d1.title, "Use Rust");
        assert_eq!(d1.status, "accepted");
        assert!(d1.description.as_deref().unwrap().starts_with("We need memory safety"));
        assert_eq!(inferred.decisions[1].status, "proposed");

        assert_eq!(
            inferred.conventions,
            vec!["Tests use tempdir.", "Never commit secrets."]
        );
        // Unreleased is skipped; versions get the v-prefix.
        assert_eq!(inferred.milestones, vec!["v1.2.0", "v1.0.0"]);
    }

    #[test]
    fn readme_blockquotes_and_emphasis_are_stripped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"p\"\n").unwrap();
        std::fs::write(
            dir.path().join("README.md"),
            "# p\n\n> One API key. One endpoint. Simple as that.\n\nBody here.\n",
        )
        .unwrap();
        let inferred = infer_identity(dir.path());
        // Two-sentence cap applies; the point is no leading "> " artifact.
        assert_eq!(
            inferred.description.as_deref(),
            Some("One API key. One endpoint.")
        );
    }

    #[test]
    fn empty_docs_mine_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let inferred = infer_identity(dir.path());
        assert!(inferred.decisions.is_empty());
        assert!(inferred.conventions.is_empty());
        assert!(inferred.milestones.is_empty());
    }
}

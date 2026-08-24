//! Repository intelligence — languages, frameworks, entry points, patterns.
//!
//! Everything produced here is **evidence-based**: a language exists because
//! source files of that language were scanned; a framework exists only when
//! a concrete dependency or manifest key proves it; an entry point exists
//! only when a manifest declares one or a language convention file is
//! present. No guessing.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use codebro_fact_store::engineering_facts::{
    EntryPointFact, EntryPointId, EntryPointKind, FrameworkFact, FrameworkId, LanguageFact,
    LanguageId, PackageId,
};

/// A dependency summary the intelligence layer needs per package.
pub struct PackageDeps {
    pub id: PackageId,
    pub name: String,
    pub language: String,
    /// Absolute directory of the package manifest.
    pub path: PathBuf,
    /// Dependency names (not versions) declared by this package.
    pub dep_names: Vec<String>,
}

/// Per-language aggregate surface: (file_count, line_count).
pub type LangStats = BTreeMap<String, (u64, u64)>;

/// Deterministic directory-skip predicate shared by all discovery walks.
pub fn skip_dir(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | ".codebro"
            | "target"
            | "node_modules"
            | "dist"
            | "build"
            | "vendor"
            | ".venv"
            | "venv"
            | "__pycache__"
            | ".mypy_cache"
            | ".pytest_cache"
            | "site-packages"
            | ".next"
            | "coverage"
    )
}

/// Aggregate language facts from scanned-file statistics.
///
/// Ids are `lang::<name>` — deterministic and stable across re-indexes.
pub fn language_facts(stats: &LangStats) -> Vec<LanguageFact> {
    stats
        .iter()
        .map(|(lang, (files, lines))| {
            let mut fact = LanguageFact::new(LanguageId::new(format!("lang::{lang}")), lang.clone());
            fact.file_count = *files;
            fact.line_count = *lines;
            fact
        })
        .collect()
}

/// Framework detection table: `(dependency name) -> (framework name)`.
/// Curated per ecosystem; deliberately conservative.
fn framework_name(ecosystem: &str, dep: &str) -> Option<&'static str> {
    let fw = match ecosystem {
        "crates.io" => match dep {
            "axum" => "Axum",
            "actix-web" => "Actix Web",
            "rocket" => "Rocket",
            "warp" => "Warp",
            "poem" => "Poem",
            "salvo" => "Salvo",
            "tokio" => "Tokio",
            "async-std" => "async-std",
            "diesel" => "Diesel",
            "sqlx" => "SQLx",
            "sea-orm" => "SeaORM",
            "clap" => "Clap",
            "ratatui" => "ratatui",
            "rmcp" => "RMCP",
            _ => return None,
        },
        "go-modules" => match dep {
            "github.com/gin-gonic/gin" => "Gin",
            "github.com/labstack/echo" => "Echo",
            "github.com/gorilla/mux" => "Gorilla Mux",
            "google.golang.org/grpc" => "gRPC",
            "github.com/spf13/cobra" => "Cobra",
            _ => return None,
        },
        "npm" => match dep {
            "express" => "Express",
            "fastify" => "Fastify",
            "koa" => "Koa",
            "hono" => "Hono",
            "next" => "Next.js",
            "nuxt" => "Nuxt.js",
            "react" => "React",
            "vue" => "Vue.js",
            "svelte" => "Svelte",
            "@angular/core" => "Angular",
            "@nestjs/core" => "NestJS",
            _ => return None,
        },
        "pypi" => match dep {
            "fastapi" => "FastAPI",
            "flask" => "Flask",
            "django" => "Django",
            "starlette" => "Starlette",
            "tornado" => "Tornado",
            "celery" => "Celery",
            _ => return None,
        },
        _ => return None,
    };
    Some(fw)
}

/// Map a package's primary language to its package-manager ecosystem.
fn ecosystem_for_language(language: &str) -> Option<&'static str> {
    match language {
        "rust" => Some("crates.io"),
        "go" => Some("go-modules"),
        "javascript" | "typescript" => Some("npm"),
        "python" => Some("pypi"),
        _ => None,
    }
}

/// Framework facts with concrete evidence. One record per distinct
/// `(ecosystem, evidence-dependency)` pair; ids are `fw::<dep-name>`.
pub fn framework_facts(packages: &[PackageDeps]) -> Vec<FrameworkFact> {
    // (evidence dep name, ecosystem) -> framework fact
    let mut by_evidence: BTreeMap<(String, String), FrameworkFact> = BTreeMap::new();
    for pkg in packages {
        let Some(ecosystem) = ecosystem_for_language(&pkg.language) else {
            continue;
        };
        for dep in &pkg.dep_names {
            let Some(fw_name) = framework_name(ecosystem, dep) else {
                continue;
            };
            let key = (dep.clone(), ecosystem.to_string());
            by_evidence
                .entry(key)
                .or_insert_with(|| {
                    let mut fact = FrameworkFact::new(
                        FrameworkId::new(format!("fw::{}", slug(dep))),
                        fw_name,
                        ecosystem,
                        dep.clone(),
                    );
                    fact.scope_package = Some(pkg.id.clone());
                    fact
                });
        }
    }
    by_evidence.into_values().collect()
}

fn slug(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .to_lowercase()
}

/// Entry-point facts from manifests and language conventions.
///
/// - Rust packages: `src/main.rs` and `src/bin/*.rs`
/// - Go packages: `main.go` and `cmd/*/main.go` containing `func main`
/// - Node packages: `package.json` `bin` / `main`
/// - Python projects: `[project.scripts]` and `__main__.py`
pub fn entry_point_facts(root: &Path, packages: &[PackageDeps]) -> Vec<EntryPointFact> {
    let mut out: BTreeMap<String, EntryPointFact> = BTreeMap::new();
    for pkg in packages {
        let pkg_dir = pkg.path.clone();
        let rel = |p: &Path| p
            .strip_prefix(root)
            .unwrap_or(p)
            .to_string_lossy()
            .to_string();

        match pkg.language.as_str() {
            "rust" => {
                let main = pkg_dir.join("src/main.rs");
                if main.is_file() {
                    insert_entry(
                                        &mut out,
                                        pkg,
                                        EntryPointFact::new(
                            EntryPointId::new(format!("entry::{}", rel(&main))),
                            pkg.name.clone(),
                            rel(&main),
                            EntryPointKind::Binary,
                            "rust",
                        ),
                    );
                }
                let bins = pkg_dir.join("src/bin");
                if bins.is_dir() {
                    for f in bin_entries(&bins, "rs") {
                        insert_entry(
                                        &mut out,
                                        pkg,
                                        EntryPointFact::new(
                                EntryPointId::new(format!("entry::{}", rel(&f))),
                                stem(&f),
                                rel(&f),
                                EntryPointKind::Binary,
                                "rust",
                            ),
                        );
                    }
                }
            }
            "go" => {
                let mut candidates = vec![pkg_dir.join("main.go")];
                let cmd = pkg_dir.join("cmd");
                if cmd.is_dir() {
                    if let Ok(rd) = std::fs::read_dir(&cmd) {
                        for e in rd.flatten() {
                            candidates.push(e.path().join("main.go"));
                        }
                    }
                }
                for cand in candidates {
                    if cand.is_file() {
                        if let Ok(text) = std::fs::read_to_string(&cand) {
                            if text.contains("func main") {
                                insert_entry(
                                        &mut out,
                                        pkg,
                                        EntryPointFact::new(
                                        EntryPointId::new(format!("entry::{}", rel(&cand))),
                                        stem(&cand),
                                        rel(&cand),
                                        EntryPointKind::Binary,
                                        "go",
                                    ),
                                );
                            }
                        }
                    }
                }
            }
            "javascript" | "typescript" => {
                let pj = pkg_dir.join("package.json");
                if let Ok(text) = std::fs::read_to_string(&pj) {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                        if let Some(bin) = value.get("bin") {
                            match bin {
                                serde_json::Value::String(path) => {
                                    let p = pkg_dir.join(path.as_str());
                                    if p.is_file() {
                                        insert_entry(&mut out, pkg,
                                            EntryPointFact::new(
                                                EntryPointId::new(format!("entry::{}", rel(&p))),
                                                pkg.name.clone(),
                                                rel(&p),
                                                EntryPointKind::Binary,
                                                pkg.language.clone(),
                                            ));
                                    }
                                }
                                serde_json::Value::Object(map) => {
                                    for (bin_name, path) in map {
                                        if let Some(ps) = path.as_str() {
                                            let p = pkg_dir.join(ps);
                                            if p.is_file() {
                                                insert_entry(&mut out, pkg,
                                                    EntryPointFact::new(
                                                        EntryPointId::new(format!("entry::{}", rel(&p))),
                                                        bin_name.clone(),
                                                        rel(&p),
                                                        EntryPointKind::Binary,
                                                        pkg.language.clone(),
                                                    ));
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        if let Some(main) = value.get("main").and_then(|m| m.as_str()) {
                            let p = pkg_dir.join(main);
                            if p.is_file() {
                                insert_entry(
                                        &mut out,
                                        pkg,
                                        EntryPointFact::new(
                                        EntryPointId::new(format!("entry::{}", rel(&p))),
                                        "main",
                                        rel(&p),
                                        EntryPointKind::Script,
                                        pkg.language.clone(),
                                    ),
                                );
                            }
                        }
                    }
                }
            }
            "python" => {
                // [project.scripts] in pyproject.toml
                let pyproject = pkg_dir.join("pyproject.toml");
                if let Ok(text) = std::fs::read_to_string(&pyproject) {
                    if let Ok(value) = text.parse::<toml::Value>() {
                        if let Some(scripts) = value
                            .get("project")
                            .and_then(|p| p.get("scripts"))
                            .and_then(|s| s.as_table())
                        {
                            for name in scripts.keys() {
                                // Script target points at module:function; the
                                // entry record names the script and cites the
                                // pyproject as its path evidence.
                                insert_entry(
                                        &mut out,
                                        pkg,
                                        EntryPointFact::new(
                                        EntryPointId::new(format!("entry::script::{}", slug(name))),
                                        name.clone(),
                                        rel(&pyproject),
                                        EntryPointKind::Script,
                                        "python",
                                    ),
                                );
                            }
                        }
                    }
                }
                // __main__.py modules (bounded search depth 3).
                for p in find_files(&pkg_dir, "__main__.py", 3) {
                    insert_entry(
                                        &mut out,
                                        pkg,
                                        EntryPointFact::new(
                            EntryPointId::new(format!("entry::{}", rel(&p))),
                            "__main__",
                            rel(&p),
                            EntryPointKind::Script,
                            "python",
                        ),
                    );
                }
            }
            _ => {}
        }
    }
    out.into_values().collect()
}

fn insert_entry(
    out: &mut BTreeMap<String, EntryPointFact>,
    pkg: &PackageDeps,
    mut fact: EntryPointFact,
) {
    // Attach the owning-package scope projection; entries are keyed by id so
    // duplicate detections dedupe deterministically.
    fact.package = Some(pkg.id.clone());
    out.entry(fact.id.as_str().to_string())
        .or_insert(fact);
}

fn bin_entries(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_file() && p.extension().and_then(|x| x.to_str()) == Some(ext) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn find_files(dir: &Path, name: &str, max_depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    fn walk(dir: &Path, name: &str, depth: usize, max_depth: usize, out: &mut Vec<PathBuf>) {
        if depth > max_depth {
            return;
        }
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                let fname = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if skip_dir(fname) {
                    continue;
                }
                if p.is_file() && fname == name {
                    out.push(p);
                } else if p.is_dir() {
                    walk(&p, name, depth + 1, max_depth, out);
                }
            }
        }
    }
    walk(dir, name, 0, max_depth, &mut out);
    out.sort();
    out
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string()
}

/// Directory-shape architecture patterns, detected deterministically.
pub fn detect_patterns(
    root: &Path,
    packages: &[PackageDeps],
    lang_stats: &LangStats,
) -> Vec<String> {
    let mut patterns = Vec::new();
    let cargo_pkgs = packages.iter().filter(|p| p.language == "rust").count();
    if cargo_pkgs > 1 && root.join("Cargo.toml").is_file() {
        patterns.push("cargo-workspace".to_string());
    }
    let ecosystems: std::collections::BTreeSet<_> = packages
        .iter()
        .filter_map(|p| ecosystem_for_language(&p.language))
        .collect();
    if ecosystems.len() > 1 {
        patterns.push("polyglot-monorepo".to_string());
    }
    if lang_stats.len() > 1 {
        patterns.push("multi-language".to_string());
    }
    if root.join("cmd").is_dir() || root.join("src/bin").is_dir() {
        patterns.push("multi-binary-layout".to_string());
    }
    patterns.sort();
    patterns.dedup();
    patterns
}

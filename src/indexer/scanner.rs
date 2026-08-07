#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::error::{CodeBroError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntry {
    pub path: String,
    pub language: String,
    pub size: u64,
    pub last_modified: String,
    pub hash: String,
    pub ignored: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RepositoryIndex {
    pub entries: Vec<IndexEntry>,
    pub root: PathBuf,
    pub generated_at: String,
}

impl RepositoryIndex {
    pub fn new(root: PathBuf) -> Self {
        RepositoryIndex {
            entries: Vec::new(),
            root,
            generated_at: chrono::Local::now().to_rfc3339(),
        }
    }

    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = fs::read_to_string(&path)
            .map_err(|e| CodeBroError::Index(format!("Failed to read index: {}", e)))?;
        let index: RepositoryIndex =
            serde_json::from_str(&content).map_err(|e| CodeBroError::Index(format!("{}", e)))?;
        Ok(index)
    }

    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        if let Some(parent) = path.as_ref().parent() {
            fs::create_dir_all(parent).map_err(|e| CodeBroError::Index(format!("{}", e)))?;
        }
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| CodeBroError::Index(format!("{}", e)))?;
        fs::write(path, content).map_err(|e| CodeBroError::Index(format!("{}", e)))?;
        Ok(())
    }

    pub fn upsert(&mut self, entry: IndexEntry) {
        if let Some(existing) = self.entries.iter_mut().find(|e| e.path == entry.path) {
            *existing = entry;
        } else {
            self.entries.push(entry);
        }
    }
}

pub struct Indexer {
    root: PathBuf,
    ignore_patterns: HashSet<String>,
    index: RepositoryIndex,
}

impl Indexer {
    pub fn new(root: PathBuf) -> Result<Self> {
        let ignore_patterns = Self::build_ignore_patterns(&root);
        let index = Self::load_existing_index(&root)?;

        Ok(Indexer {
            root,
            ignore_patterns,
            index,
        })
    }

    pub fn index(&mut self) -> Result<&RepositoryIndex> {
        let gitignore_path = self.root.join(".gitignore");
        let gitignore_patterns = if gitignore_path.exists() {
            Self::parse_gitignore(&gitignore_path)?
        } else {
            HashSet::new()
        };

        let default_ignores: HashSet<String> = vec![
            "target/",
            "node_modules/",
            "dist/",
            "build/",
            "vendor/",
            ".git/",
            ".codebro/",
            "*.rs.bk",
            "*.swp",
            "*.swo",
            "*~",
            "*.tmp",
            ".DS_Store",
            "Thumbs.db",
            "*.pyc",
            "__pycache__/",
            ".pytest_cache/",
            ".venv/",
            "venv/",
            ".mypy_cache/",
            ".ruff_cache/",
            "Cargo.lock",
            "package-lock.json",
            "yarn.lock",
            "pnpm-lock.yaml",
        ]
        .into_iter()
        .map(|s| s.to_string())
        .collect();

        let mut all_ignores = self.ignore_patterns.clone();
        all_ignores.extend(gitignore_patterns);
        all_ignores.extend(default_ignores);

        for entry in walkdir::WalkDir::new(&self.root)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            let relative = path
                .strip_prefix(&self.root)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string();

            if relative.is_empty() || relative == "." {
                continue;
            }

            if Self::is_ignored(&relative, &all_ignores) {
                continue;
            }

            if entry.file_type().is_dir() {
                continue;
            }

            if Self::is_binary(path) {
                continue;
            }

            let metadata = match fs::metadata(path) {
                Ok(m) => m,
                Err(_) => continue,
            };

            let size = metadata.len();
            let last_modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let last_modified_str =
                chrono::DateTime::<chrono::Local>::from(last_modified).to_rfc3339();

            let content_hash = Self::compute_hash(path).unwrap_or_default();

            let language = Self::detect_language(path);

            let entry = IndexEntry {
                path: relative,
                language,
                size,
                last_modified: last_modified_str,
                hash: content_hash,
                ignored: false,
            };

            self.index.upsert(entry);
        }

        self.index.generated_at = chrono::Local::now().to_rfc3339();
        Ok(&self.index)
    }

    pub fn incremental_refresh(&mut self) -> Result<()> {
        for entry in walkdir::WalkDir::new(&self.root)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            let relative = path
                .strip_prefix(&self.root)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string();

            if relative.is_empty() || relative == "." || entry.file_type().is_dir() {
                continue;
            }

            if Self::is_binary(path) {
                continue;
            }

            let metadata = match fs::metadata(path) {
                Ok(m) => m,
                Err(_) => continue,
            };

            let size = metadata.len();
            let last_modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let last_modified_str =
                chrono::DateTime::<chrono::Local>::from(last_modified).to_rfc3339();
            let content_hash = Self::compute_hash(path).unwrap_or_default();

            if let Some(existing) = self.index.entries.iter().find(|e| e.path == relative) {
                if existing.hash == content_hash && existing.size == size {
                    continue;
                }
            }

            let language = Self::detect_language(path);

            self.index.upsert(IndexEntry {
                path: relative,
                language,
                size,
                last_modified: last_modified_str,
                hash: content_hash,
                ignored: false,
            });
        }

        self.index.generated_at = chrono::Local::now().to_rfc3339();
        Ok(())
    }

    pub fn index_path(&self) -> PathBuf {
        self.root.join(".codebro").join("index.json")
    }

    pub fn save_index(&self) -> Result<()> {
        self.index.save(self.index_path())
    }

    pub fn into_index(self) -> RepositoryIndex {
        self.index
    }

    fn load_existing_index(root: &Path) -> Result<RepositoryIndex> {
        let index_path = root.join(".codebro").join("index.json");
        if index_path.exists() {
            RepositoryIndex::load(index_path)
        } else {
            Ok(RepositoryIndex::new(root.to_path_buf()))
        }
    }

    fn build_ignore_patterns(root: &Path) -> HashSet<String> {
        let mut patterns = HashSet::new();
        let gitignore_path = root.join(".gitignore");
        if let Ok(pats) = Self::parse_gitignore(&gitignore_path) {
            patterns = pats;
        }
        patterns
    }

    fn parse_gitignore(path: &Path) -> Result<HashSet<String>> {
        let mut patterns = HashSet::new();
        if !path.exists() {
            return Ok(patterns);
        }

        let content = fs::read_to_string(path)
            .map_err(|e| CodeBroError::Index(format!("Failed to read .gitignore: {}", e)))?;

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            patterns.insert(line.to_string());
        }

        Ok(patterns)
    }

    fn is_ignored(relative: &str, patterns: &HashSet<String>) -> bool {
        let path_str = relative.to_string();

        for pattern in patterns {
            if pattern.ends_with('/') {
                let dir_pattern = pattern.trim_end_matches('/');
                if path_str.starts_with(dir_pattern) || path_str == dir_pattern {
                    return true;
                }
            } else if pattern.starts_with('.') && !pattern.contains('*') {
                let hidden = relative.split('/').any(|part| part == pattern);
                if hidden {
                    return true;
                }
            } else if pattern.contains('*') {
                let regex_pattern = pattern
                    .replace(".", "\\.")
                    .replace("*", ".*")
                    .replace("?", ".");
                if let Ok(re) = regex::Regex::new(&format!("^{}$", regex_pattern)) {
                    if re.is_match(relative) {
                        return true;
                    }
                }
            } else {
                if relative == *pattern
                    || relative.starts_with(&format!("{}/", pattern))
                    || relative.starts_with(&format!("{}/", pattern))
                {
                    return true;
                }
            }
        }

        false
    }

    fn is_binary(path: &Path) -> bool {
        if let Some(ext) = path.extension() {
            let ext = ext.to_string_lossy().to_lowercase();
            let binary_exts = vec![
                "bin", "exe", "dll", "so", "dylib", "o", "a", "lib", "pyc", "png", "jpg", "jpeg",
                "gif", "ico", "bmp", "svg", "zip", "tar", "gz", "bz2", "xz", "7z", "rar", "mp3",
                "mp4", "avi", "mov", "flac", "wav", "ogg", "pdf", "doc", "docx", "xls", "xlsx",
                "ppt", "pptx", "sqlite", "db", "sqlite3",
            ];
            if binary_exts.contains(&ext.as_str()) {
                return true;
            }
        }

        if let Ok(content) = fs::read(path) {
            if content.len() > 1024 {
                let sample = &content[..1024];
                let non_text = sample
                    .iter()
                    .filter(|&&b| b < 32 && !b.is_ascii_whitespace())
                    .count();
                if non_text > sample.len() / 10 {
                    return true;
                }
            }
        }

        false
    }

    fn compute_hash(path: &Path) -> Result<String> {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let content = fs::read(path)
            .map_err(|e| CodeBroError::Index(format!("Failed to read file for hash: {}", e)))?;

        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        Ok(format!("{:x}", hasher.finish()))
    }

    fn detect_language(path: &Path) -> String {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        match ext {
            "rs" => "rust".to_string(),
            "js" | "mjs" => "javascript".to_string(),
            "ts" | "mts" => "typescript".to_string(),
            "tsx" => "tsx".to_string(),
            "jsx" => "jsx".to_string(),
            "py" => "python".to_string(),
            "go" => "go".to_string(),
            "java" => "java".to_string(),
            "c" => "c".to_string(),
            "cpp" | "cc" | "cxx" => "cpp".to_string(),
            "h" | "hpp" => "c-header".to_string(),
            "rb" => "ruby".to_string(),
            "php" => "php".to_string(),
            "swift" => "swift".to_string(),
            "kt" | "kts" => "kotlin".to_string(),
            "scala" => "scala".to_string(),
            "html" => "html".to_string(),
            "css" | "scss" | "sass" => "css".to_string(),
            "json" => "json".to_string(),
            "yaml" | "yml" => "yaml".to_string(),
            "toml" => "toml".to_string(),
            "md" | "markdown" => "markdown".to_string(),
            "sh" | "bash" | "zsh" => "shell".to_string(),
            "sql" => "sql".to_string(),
            "tf" | "hcl" => "terraform".to_string(),
            "xml" => "xml".to_string(),
            _ => "unknown".to_string(),
        }
    }
}

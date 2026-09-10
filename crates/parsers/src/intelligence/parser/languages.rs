#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use tree_sitter::Language;

pub fn get_language(name: &str) -> Option<Language> {
    match name {
        "rust" => Some(tree_sitter_rust::language()),
        "python" => Some(tree_sitter_python::language()),
        "javascript" => Some(tree_sitter_javascript::language()),
        "typescript" => Some(tree_sitter_typescript::language_typescript()),
        "jsx" | "tsx" => Some(tree_sitter_typescript::language_tsx()),
        "go" => Some(tree_sitter_go::language()),
        _ => None,
    }
}

pub fn get_supported_languages() -> Vec<&'static str> {
    vec![
        "rust",
        "python",
        "javascript",
        "typescript",
        "tsx",
        "jsx",
        "go",
    ]
}

pub fn language_from_extension(ext: &str) -> Option<&'static str> {
    match ext {
        "rs" => Some("rust"),
        "py" => Some("python"),
        "js" => Some("javascript"),
        "ts" => Some("typescript"),
        "tsx" => Some("tsx"),
        "jsx" => Some("jsx"),
        "go" => Some("go"),
        _ => None,
    }
}

/// P6 file-level language detection: broader than the tree-sitter parser
/// surface. Returns a language label for every file CodeBro understands
/// at the *file* level, even when no symbol extraction exists. `None`
/// means genuinely unknown (binary, unrecognised extension).
///
/// Supported file-level languages: rust, python, javascript, typescript,
/// tsx, jsx, go (parsed) + c, cpp, shell, toml, yaml, json, markdown
/// (file intelligence only — no invented symbols).
pub fn file_language_from_extension(ext: &str) -> Option<&'static str> {
    if let Some(parsed) = language_from_extension(ext) {
        return Some(parsed);
    }
    match ext {
        "c" | "h" => Some("c"),
        "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => Some("cpp"),
        "sh" | "bash" | "zsh" => Some("shell"),
        "toml" => Some("toml"),
        "yaml" | "yml" => Some("yaml"),
        "json" => Some("json"),
        "md" | "markdown" => Some("markdown"),
        _ => None,
    }
}

/// True when tree-sitter symbol extraction exists for this file-level
/// language. `c`/`cpp`/`shell`/config formats return false: CodeBro
/// preserves file-level intelligence for them but never invents symbols.
pub fn is_parser_supported(file_language: &str) -> bool {
    matches!(
        file_language,
        "rust" | "python" | "javascript" | "typescript" | "tsx" | "jsx" | "go"
    )
}

/// Human-readable parser limitation for file-level-only languages.
/// Returns `None` when full symbol support exists or the language is unknown.
pub fn parser_limitation(file_language: &str) -> Option<&'static str> {
    match file_language {
        "c" | "cpp" | "shell" => Some("file-level only: no symbol extraction for this language"),
        "toml" | "yaml" | "json" | "markdown" => {
            Some("file-level only: configuration/documentation, no code symbols")
        }
        _ => None,
    }
}

pub fn get_tree_sitter_language_name(language: &str) -> &'static str {
    match language {
        "rust" => "rust",
        "python" => "python",
        "javascript" | "jsx" => "javascript",
        "typescript" | "tsx" => "typescript",
        "go" => "go",
        _ => "unknown",
    }
}

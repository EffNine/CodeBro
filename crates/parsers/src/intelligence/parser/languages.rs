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

#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use anyhow::{Context, Result};
use walkdir::WalkDir;

pub struct ListFiles;

impl super::Tool for ListFiles {
    fn name(&self) -> &str {
        "list_files"
    }

    fn description(&self) -> &str {
        "List files in a directory"
    }

    fn execute(&self, args: &str) -> Result<String> {
        let path = if args.is_empty() { "." } else { args };
        let mut files = Vec::new();

        for entry in WalkDir::new(path).max_depth(2).into_iter().flatten() {
            if entry.file_type().is_file() {
                files.push(entry.path().display().to_string());
            }
        }

        Ok(files.join("\n"))
    }
}

pub struct ReadFile;

impl super::Tool for ReadFile {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read the contents of a file"
    }

    fn execute(&self, args: &str) -> Result<String> {
        let content = std::fs::read_to_string(args)
            .with_context(|| format!("Failed to read file: {}", args))?;
        Ok(content)
    }
}

pub struct CreateFile;

impl super::Tool for CreateFile {
    fn name(&self) -> &str {
        "create_file"
    }

    fn description(&self) -> &str {
        "Create a new file with content"
    }

    fn execute(&self, args: &str) -> Result<String> {
        let parts: Vec<&str> = args.splitn(2, '|').collect();
        if parts.len() < 2 {
            return Err(anyhow::anyhow!("Usage: create_file <path>|<content>"));
        }
        let path = parts[0].trim();
        let content = parts[1].trim();
        std::fs::write(path, content)
            .with_context(|| format!("Failed to create file: {}", path))?;
        Ok(format!("Created file: {}", path))
    }
}

pub struct EditFile;

impl super::Tool for EditFile {
    fn name(&self) -> &str {
        "edit_file"
    }

    fn description(&self) -> &str {
        "Edit a file by replacing old text with new text"
    }

    fn execute(&self, args: &str) -> Result<String> {
        let parts: Vec<&str> = args.splitn(3, '|').collect();
        if parts.len() < 3 {
            return Err(anyhow::anyhow!(
                "Usage: edit_file <path>|<old_text>|<new_text>"
            ));
        }
        let path = parts[0].trim();
        let old_text = parts[1].trim();
        let new_text = parts[2].trim();

        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read file: {}", path))?;

        if !content.contains(old_text) {
            return Err(anyhow::anyhow!("Text not found in file: {}", old_text));
        }

        let new_content = content.replace(old_text, new_text);
        std::fs::write(path, new_content)
            .with_context(|| format!("Failed to write file: {}", path))?;

        Ok(format!("Edited file: {}", path))
    }
}

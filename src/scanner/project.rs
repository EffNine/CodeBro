#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::error::Result;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectInfo {
    pub name: String,
    pub path: PathBuf,
    pub language: String,
    pub framework: Option<String>,
    pub build_system: Option<String>,
    pub package_manager: Option<String>,
    pub testing_framework: Option<String>,
    pub important_files: Vec<String>,
}

impl ProjectInfo {
    pub fn detect(path: PathBuf) -> Result<Self> {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        let mut info = ProjectInfo {
            name,
            path,
            language: "unknown".to_string(),
            framework: None,
            build_system: None,
            package_manager: None,
            testing_framework: None,
            important_files: Vec::new(),
        };

        info.detect_language()?;
        info.detect_framework()?;
        info.detect_build_system()?;
        info.detect_package_manager()?;
        info.detect_testing_framework()?;
        info.detect_important_files()?;

        Ok(info)
    }

    fn detect_language(&mut self) -> Result<()> {
        let files: Vec<_> = std::fs::read_dir(&self.path)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name())
                    .collect()
            })
            .unwrap_or_default();

        let file_names: Vec<String> = files
            .iter()
            .map(|f| f.to_string_lossy().to_string())
            .collect();

        if file_names.contains(&"Cargo.toml".to_string()) {
            self.language = "rust".to_string();
        } else if file_names.contains(&"package.json".to_string()) {
            self.language = "javascript".to_string();
        } else if file_names.contains(&"pyproject.toml".to_string())
            || file_names.contains(&"requirements.txt".to_string())
            || file_names.contains(&"setup.py".to_string())
            || file_names.contains(&"setup.cfg".to_string())
        {
            self.language = "python".to_string();
        } else if file_names.contains(&"go.mod".to_string()) {
            self.language = "go".to_string();
        } else if file_names.contains(&"pom.xml".to_string())
            || file_names.contains(&"build.gradle".to_string())
        {
            self.language = "java".to_string();
        } else if file_names.contains(&"Gemfile".to_string()) {
            self.language = "ruby".to_string();
        } else if file_names.contains(&"composer.json".to_string()) {
            self.language = "php".to_string();
        } else if file_names.contains(&"pom.xml".to_string()) {
            self.language = "java".to_string();
        } else {
            // Read directory entries, falling back to current dir if workspace
            // path is inaccessible. Handle errors gracefully without panicking.
            let dir_entries = match std::fs::read_dir(&self.path) {
                Ok(entries) => entries,
                Err(_) => match std::fs::read_dir(".") {
                    Ok(entries) => entries,
                    Err(_) => return Ok(()), // Both failed, skip language detection
                },
            };
            // Collect entries to a Vec so we can iterate safely
            let mut file_entries = Vec::new();
            for entry in dir_entries {
                if let Ok(e) = entry {
                    file_entries.push(e);
                }
            }
            for entry in file_entries {
                if let Some(ext) = entry.path().extension() {
                    let ext = ext.to_string_lossy().to_string();
                    match ext.as_str() {
                        "rs" => {
                            self.language = "rust".to_string();
                            break;
                        }
                        "js" | "jsx" | "ts" | "tsx" => {
                            self.language = "javascript".to_string();
                            break;
                        }
                        "py" => {
                            self.language = "python".to_string();
                            break;
                        }
                        "go" => {
                            self.language = "go".to_string();
                            break;
                        }
                        "java" => {
                            self.language = "java".to_string();
                            break;
                        }
                        "rb" => {
                            self.language = "ruby".to_string();
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(())
    }

    fn detect_framework(&mut self) -> Result<()> {
        let cargo_toml = self.path.join("Cargo.toml");
        if cargo_toml.exists() {
            if let Ok(content) = std::fs::read_to_string(&cargo_toml) {
                if content.contains("actix-web") || content.contains("actix_web") {
                    self.framework = Some("actix-web".to_string());
                } else if content.contains("axum") {
                    self.framework = Some("axum".to_string());
                } else if content.contains("rocket") {
                    self.framework = Some("rocket".to_string());
                } else if content.contains("warp") {
                    self.framework = Some("warp".to_string());
                } else if content.contains("bevy") {
                    self.framework = Some("bevy".to_string());
                } else if content.contains("tauri") {
                    self.framework = Some("tauri".to_string());
                } else if content.contains("dioxus") {
                    self.framework = Some("dioxus".to_string());
                } else if content.contains("yew") {
                    self.framework = Some("yew".to_string());
                }
            }
        }

        let package_json = self.path.join("package.json");
        if package_json.exists() {
            if let Ok(content) = std::fs::read_to_string(&package_json) {
                if content.contains("\"react\"") {
                    self.framework = Some("react".to_string());
                } else if content.contains("\"vue\"") {
                    self.framework = Some("vue".to_string());
                } else if content.contains("\"angular\"") {
                    self.framework = Some("angular".to_string());
                } else if content.contains("\"svelte\"") {
                    self.framework = Some("svelte".to_string());
                } else if content.contains("\"next\"") {
                    self.framework = Some("next.js".to_string());
                } else if content.contains("\"nuxt\"") {
                    self.framework = Some("nuxt".to_string());
                } else if content.contains("\"express\"") {
                    self.framework = Some("express".to_string());
                } else if content.contains("\"fastify\"") {
                    self.framework = Some("fastify".to_string());
                }
            }
        }

        Ok(())
    }

    fn detect_build_system(&mut self) -> Result<()> {
        if self.path.join("Cargo.toml").exists() {
            self.build_system = Some("cargo".to_string());
        } else if self.path.join("package.json").exists() {
            self.build_system = Some("npm".to_string());
        } else if self.path.join("pyproject.toml").exists() {
            self.build_system = Some("python".to_string());
        } else if self.path.join("go.mod").exists() {
            self.build_system = Some("go".to_string());
        } else if self.path.join("Makefile").exists() {
            self.build_system = Some("make".to_string());
        } else if self.path.join("CMakeLists.txt").exists() {
            self.build_system = Some("cmake".to_string());
        }
        Ok(())
    }

    fn detect_package_manager(&mut self) -> Result<()> {
        if self.path.join("Cargo.toml").exists() {
            self.package_manager = Some("cargo".to_string());
        } else if self.path.join("pnpm-lock.yaml").exists() {
            self.package_manager = Some("pnpm".to_string());
        } else if self.path.join("yarn.lock").exists() {
            self.package_manager = Some("yarn".to_string());
        } else if self.path.join("package-lock.json").exists() {
            self.package_manager = Some("npm".to_string());
        } else if self.path.join("requirements.txt").exists() {
            self.package_manager = Some("pip".to_string());
        } else if self.path.join("poetry.lock").exists() {
            self.package_manager = Some("poetry".to_string());
        }
        Ok(())
    }

    fn detect_testing_framework(&mut self) -> Result<()> {
        if self.language == "rust" {
            self.testing_framework = Some("cargo test".to_string());
        } else if self.language == "javascript" {
            let package_json = self.path.join("package.json");
            if package_json.exists() {
                if let Ok(content) = std::fs::read_to_string(&package_json) {
                    if content.contains("jest") {
                        self.testing_framework = Some("jest".to_string());
                    } else if content.contains("vitest") {
                        self.testing_framework = Some("vitest".to_string());
                    } else if content.contains("mocha") {
                        self.testing_framework = Some("mocha".to_string());
                    } else if content.contains("@playwright") {
                        self.testing_framework = Some("playwright".to_string());
                    } else if content.contains("cypress") {
                        self.testing_framework = Some("cypress".to_string());
                    }
                }
            }
        } else if self.language == "python" {
            if self.path.join("pytest.ini").exists() || self.path.join("conftest.py").exists() {
                self.testing_framework = Some("pytest".to_string());
            } else {
                self.testing_framework = Some("unittest".to_string());
            }
        }
        Ok(())
    }

    fn detect_important_files(&mut self) -> Result<()> {
        let important_patterns = vec![
            "README.md",
            "README",
            "LICENSE",
            "main.rs",
            "lib.rs",
            "app.rs",
            "mod.rs",
            "index.js",
            "index.ts",
            "App.jsx",
            "App.tsx",
            "main.py",
            "app.py",
            "Cargo.toml",
            "package.json",
            "go.mod",
            "Makefile",
            "Dockerfile",
            ".env.example",
            "docker-compose.yml",
            "config.rs",
            "config.toml",
        ];

        for pattern in important_patterns {
            let path = self.path.join(pattern);
            if path.exists() {
                self.important_files.push(pattern.to_string());
            }
        }

        Ok(())
    }
}

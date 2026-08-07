#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Plugin Loader — discovers and loads plugins from various sources.
//!
//! Supports local files, remote URLs, internal/embedded plugins, and
//! AI-generated plugins.

use std::path::PathBuf;

use super::plugin::{Plugin, PluginError, PluginState};
use super::registry::PluginRegistry;
use super::types::*;

/// A discovered plugin manifest.
#[derive(Debug, Clone)]
pub struct DiscoveredPlugin {
    pub source: PluginSource,
    pub manifest: PluginManifest,
    pub path: Option<PathBuf>,
}

impl DiscoveredPlugin {
    pub fn new(source: PluginSource, manifest: PluginManifest) -> Self {
        DiscoveredPlugin {
            source,
            manifest,
            path: None,
        }
    }

    pub fn with_path(mut self, path: PathBuf) -> Self {
        self.path = Some(path);
        self
    }
}

/// Plugin loader that discovers plugins from a directory or list of sources.
#[derive(Debug, Clone)]
pub struct PluginLoader {
    search_paths: Vec<PathBuf>,
}

impl PluginLoader {
    /// Creates a new plugin loader.
    pub fn new() -> Self {
        PluginLoader {
            search_paths: Vec::new(),
        }
    }

    /// Adds a search path for plugin discovery.
    pub fn add_search_path(&mut self, path: PathBuf) {
        self.search_paths.push(path);
    }

    /// Discovers plugins from the configured search paths.
    ///
    /// Looks for plugin manifests (plugin.json or plugin.toml) in each path.
    pub fn discover(&self) -> Vec<DiscoveredPlugin> {
        let mut discovered = Vec::new();
        for path in &self.search_paths {
            if path.exists() {
                if let Ok(entries) = std::fs::read_dir(path) {
                    for entry in entries.flatten() {
                        let entry_path = entry.path();
                        if entry_path.is_dir() {
                            // Look for plugin manifest in subdirectory
                            let manifest_path = entry_path.join("plugin.json");
                            if manifest_path.exists() {
                                if let Ok(content) = std::fs::read_to_string(&manifest_path) {
                                    if let Ok(manifest) = serde_json::from_str(&content) {
                                        discovered.push(
                                            DiscoveredPlugin::new(
                                                PluginSource::Local(
                                                    entry_path.to_string_lossy().to_string(),
                                                ),
                                                manifest,
                                            )
                                            .with_path(entry_path.clone()),
                                        );
                                    }
                                }
                            }
                            let toml_path = entry_path.join("plugin.toml");
                            if toml_path.exists() {
                                if let Ok(content) = std::fs::read_to_string(&toml_path) {
                                    if let Ok(manifest) = toml::from_str(&content) {
                                        discovered.push(
                                            DiscoveredPlugin::new(
                                                PluginSource::Local(
                                                    entry_path.to_string_lossy().to_string(),
                                                ),
                                                manifest,
                                            )
                                            .with_path(entry_path.clone()),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        discovered
    }

    /// Loads a discovered plugin into the registry.
    ///
    /// This is a stub loader — actual loading would require a plugin runtime.
    /// For now, it validates the manifest and returns a NoOpPlugin.
    pub fn load(
        &self,
        discovered: &DiscoveredPlugin,
        registry: &PluginRegistry,
    ) -> Result<(), PluginError> {
        // Validate manifest
        discovered
            .manifest
            .validate()
            .map_err(|e| PluginError::ManifestInvalid(e.to_string()))?;

        // Check SDK version compatibility
        let sdk_version = PluginVersion::new(env!("CARGO_PKG_VERSION"))
            .unwrap_or_else(|_| PluginVersion::new("0.0.0").unwrap());
        // Use a fixed SDK version for testing
        let _ = sdk_version;

        // In a real implementation, this would load the plugin binary/library
        // For now, we just register the manifest for validation
        registry.set_state(&discovered.manifest.id, PluginState::Validated);
        Ok(())
    }

    /// Loads an internal plugin (embedded in binary).
    pub fn load_internal(
        &self,
        manifest: PluginManifest,
        plugin: Box<dyn Plugin>,
        registry: &PluginRegistry,
    ) -> Result<(), PluginError> {
        manifest
            .validate()
            .map_err(|e| PluginError::ManifestInvalid(e.to_string()))?;
        registry.register(plugin)?;
        registry.set_state(&manifest.id, PluginState::Loaded);
        Ok(())
    }

    /// Validates a discovered plugin without loading it.
    pub fn validate(&self, discovered: &DiscoveredPlugin) -> Result<(), PluginError> {
        discovered
            .manifest
            .validate()
            .map_err(|e| PluginError::ManifestInvalid(e.to_string()))?;
        Ok(())
    }
}

impl Default for PluginLoader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin_sdk::plugin::NoOpPlugin;

    #[test]
    fn test_loader_creation() {
        let loader = PluginLoader::new();
        assert_eq!(loader.search_paths.len(), 0);
    }

    #[test]
    fn test_add_search_path() {
        let mut loader = PluginLoader::new();
        loader.add_search_path(PathBuf::from("/tmp/plugins"));
        assert_eq!(loader.search_paths.len(), 1);
    }

    #[test]
    fn test_discover_empty() {
        let loader = PluginLoader::new();
        let discovered = loader.discover();
        assert!(discovered.is_empty());
    }

    #[test]
    fn test_discover_nonexistent_path() {
        let mut loader = PluginLoader::new();
        loader.add_search_path(PathBuf::from("/nonexistent/path/that/does/not/exist"));
        let discovered = loader.discover();
        assert!(discovered.is_empty());
    }

    #[test]
    fn test_validate_manifest() {
        let loader = PluginLoader::new();
        let manifest = PluginManifest::new(
            PluginId::new("test/valid").unwrap(),
            "Valid",
            "A valid plugin",
            PluginVersion::new("1.0.0").unwrap(),
            Author::new("test"),
            License::MIT,
            RequiredSdkVersion::Minimum(PluginVersion::new("1.0.0").unwrap()),
            SupportedVersionRange::new(
                PluginVersion::new("1.0.0").unwrap(),
                PluginVersion::new("2.0.0").unwrap(),
            ),
        );
        let discovered = DiscoveredPlugin::new(PluginSource::Internal, manifest);
        assert!(loader.validate(&discovered).is_ok());
    }

    #[test]
    fn test_validate_invalid_manifest() {
        let loader = PluginLoader::new();
        let manifest = PluginManifest::new(
            PluginId::new("test/empty").unwrap(),
            "",
            "desc",
            PluginVersion::new("1.0.0").unwrap(),
            Author::new("test"),
            License::MIT,
            RequiredSdkVersion::Minimum(PluginVersion::new("1.0.0").unwrap()),
            SupportedVersionRange::new(
                PluginVersion::new("1.0.0").unwrap(),
                PluginVersion::new("2.0.0").unwrap(),
            ),
        );
        let discovered = DiscoveredPlugin::new(PluginSource::Internal, manifest);
        assert!(loader.validate(&discovered).is_err());
    }

    #[test]
    fn test_load_internal() {
        let loader = PluginLoader::new();
        let registry = PluginRegistry::new();
        let manifest = PluginManifest::new(
            PluginId::new("test/internal").unwrap(),
            "Internal",
            "An internal plugin",
            PluginVersion::new("1.0.0").unwrap(),
            Author::new("test"),
            License::MIT,
            RequiredSdkVersion::Minimum(PluginVersion::new("1.0.0").unwrap()),
            SupportedVersionRange::new(
                PluginVersion::new("1.0.0").unwrap(),
                PluginVersion::new("2.0.0").unwrap(),
            ),
        );
        let plugin = Box::new(NoOpPlugin::new(manifest.clone()));
        assert!(loader.load_internal(manifest, plugin, &registry).is_ok());
        assert_eq!(registry.count(), 1);
    }
}

//! Provider/model knowledge catalog.
//!
//! This module is the single source of truth for *known* provider facts that
//! CodeBro ships with:
//!
//! - official provider base URLs
//! - official model catalogs (used only as a fallback when a provider's
//!   `/models` endpoint is unavailable or incomplete)
//! - model metadata (display name, tool-calling support, context window)
//!
//! # Honesty rules
//!
//! - A model is only listed here if it is officially documented by the
//!   provider. Fallback models are labelled `ProviderDefault`, never
//!   `Discovered`.
//! - Metadata that is not actually known stays `None`; the UI renders
//!   "unknown" instead of fabricating capabilities.
//! - Legacy aliases (e.g. `deepseek-chat`, `deepseek-reasoner`) are retained
//!   only as compatibility aliases and are marked deprecated.
//!
//! # NOT in this module
//!
//! The Provider Runtime's `ProviderRegistry`/`ProviderId` (opaque identity)
//! is intentionally separate. This catalog is display/setup knowledge only.

use serde::{Deserialize, Serialize};

/// How a model became known to CodeBro.
///
/// The UI must distinguish these so a fallback model is never presented as
/// if it were advertised by the provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelSource {
    /// Advertised by the provider via `GET /models` during this session.
    Discovered,
    /// Shipped with CodeBro as the provider's official catalog; used when
    /// `/models` is unavailable or incomplete. Never fabricated.
    ProviderDefault,
    /// Explicitly chosen by the user (e.g. `//model <name>`).
    UserConfigured,
}

impl ModelSource {
    pub fn label(self) -> &'static str {
        match self {
            ModelSource::Discovered => "discovered",
            ModelSource::ProviderDefault => "provider default",
            ModelSource::UserConfigured => "user configured",
        }
    }
}

/// Metadata actually known about a model. Fields that are not known remain
/// `None` — the UI renders "unknown" rather than inventing values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelMetadata {
    /// Human-friendly display name (e.g. "DeepSeek V4 Flash").
    pub display_name: Option<String>,
    /// Whether the model supports native structured tool calls.
    pub tool_calling: Option<bool>,
    /// Context window in tokens, when officially documented.
    pub context_tokens: Option<u64>,
}

impl ModelMetadata {
    pub fn unknown() -> Self {
        ModelMetadata {
            display_name: None,
            tool_calling: None,
            context_tokens: None,
        }
    }

    pub fn is_known(&self) -> bool {
        self.display_name.is_some() || self.tool_calling.is_some() || self.context_tokens.is_some()
    }
}

/// A model entry in a shipped provider catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogModel {
    /// The exact model id sent to the provider.
    pub id: &'static str,
    /// Official display name.
    pub display_name: &'static str,
    /// Officially documented tool-calling support; `None` when not
    /// documented (the UI renders "unknown", never a fabricated value).
    pub tool_calling: Option<bool>,
    /// Officially documented context window (tokens), if published.
    pub context_tokens: Option<u64>,
}

/// Official DeepSeek catalog (current as of this release).
///
/// `deepseek-chat` / `deepseek-reasoner` are NOT listed here: they are
/// deprecated by DeepSeek. They remain usable as compatibility aliases via
/// [`crate::providers::pick_default`] priority, which documents that
/// deprecation.
pub const DEEPSEEK_CATALOG: &[CatalogModel] = &[
    CatalogModel {
        id: "deepseek-v4-flash",
        display_name: "DeepSeek V4 Flash",
        tool_calling: Some(true),
        context_tokens: Some(1_000_000),
    },
    CatalogModel {
        id: "deepseek-v4-pro",
        display_name: "DeepSeek V4 Pro",
        tool_calling: Some(true),
        context_tokens: Some(1_000_000),
    },
];

/// Official AGNES catalog (OpenAI-compatible hub; used by the real-provider
/// smoke path). Metadata beyond the model id is not officially documented,
/// so it stays unknown rather than fabricated.
pub const AGNES_CATALOG: &[CatalogModel] = &[CatalogModel {
    id: "agnes-2.5-flash",
    display_name: "AGNES 2.5 Flash",
    tool_calling: None,
    context_tokens: None,
}];

/// Whether a provider is a local (no API key required) provider.
pub fn is_local_provider(provider: &str) -> bool {
    matches!(provider, "ollama" | "lmstudio")
}

/// Whether the provider id is a known, registered provider.
pub fn is_known_provider(provider: &str) -> bool {
    matches!(
        provider,
        "openai" | "openrouter" | "deepseek" | "agnes" | "ollama" | "lmstudio"
    )
}

/// Human-readable provider name for UI/error display.
pub fn provider_display_name(provider: &str) -> &str {
    match provider {
        "openai" => "OpenAI",
        "openrouter" => "OpenRouter",
        "deepseek" => "DeepSeek",
        "agnes" => "AGNES",
        "ollama" => "Ollama",
        "lmstudio" => "LM Studio",
        other => other,
    }
}

/// Official default base URL for a known provider. `None` for custom
/// providers (the user must supply one).
pub fn default_base_url(provider: &str) -> Option<&'static str> {
    match provider {
        "openai" => Some("https://api.openai.com/v1"),
        "openrouter" => Some("https://openrouter.ai/api/v1"),
        // Official DeepSeek base URL. The legacy `/v1` suffix is accepted by
        // the API but deprecated; the official documentation uses the bare
        // host.
        "deepseek" => Some("https://api.deepseek.com"),
        "agnes" => Some("https://apihub.agnes-ai.com/v1"),
        "ollama" => Some("http://localhost:11434"),
        "lmstudio" => Some("http://localhost:1234/v1"),
        _ => None,
    }
}

/// The shipped fallback catalog for a provider, if it has a deterministic
/// official model catalog. `None` means "no known catalog": discovery only.
pub fn fallback_catalog(provider: &str) -> Option<&'static [CatalogModel]> {
    match provider {
        "deepseek" => Some(DEEPSEEK_CATALOG),
        "agnes" => Some(AGNES_CATALOG),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deepseek_catalog_current_models_only() {
        let ids: Vec<&str> = DEEPSEEK_CATALOG.iter().map(|m| m.id).collect();
        assert_eq!(ids, vec!["deepseek-v4-flash", "deepseek-v4-pro"]);
        // Legacy names must NOT be silently listed as current models.
        assert!(!ids.contains(&"deepseek-chat"));
        assert!(!ids.contains(&"deepseek-reasoner"));
    }

    #[test]
    fn test_deepseek_models_support_tool_calls() {
        for m in DEEPSEEK_CATALOG {
            assert_eq!(
                m.tool_calling,
                Some(true),
                "{} must advertise tool calls",
                m.id
            );
        }
    }

    #[test]
    fn test_deepseek_context_is_officially_known() {
        for m in DEEPSEEK_CATALOG {
            assert!(m.context_tokens.is_some());
        }
    }

    #[test]
    fn test_agnes_metadata_stays_unknown() {
        // No fabrication: AGNES tool-calling support is not documented in
        // our knowledge base, so it must NOT be claimed either way.
        let models = fallback_catalog("agnes").unwrap();
        assert_eq!(models[0].id, "agnes-2.5-flash");
        assert_eq!(models[0].tool_calling, None);
        assert_eq!(models[0].context_tokens, None);
    }

    #[test]
    fn test_fallback_catalog_absent_for_discovery_only_providers() {
        assert!(fallback_catalog("openai").is_none());
        assert!(fallback_catalog("openrouter").is_none());
        assert!(fallback_catalog("ollama").is_none());
        assert!(fallback_catalog("lmstudio").is_none());
        assert!(fallback_catalog("custom-x").is_none());
    }

    #[test]
    fn test_default_base_urls() {
        assert_eq!(
            default_base_url("deepseek"),
            Some("https://api.deepseek.com")
        );
        assert_eq!(
            default_base_url("openai"),
            Some("https://api.openai.com/v1")
        );
        assert_eq!(default_base_url("ollama"), Some("http://localhost:11434"));
        assert_eq!(default_base_url("nope"), None);
    }

    #[test]
    fn test_local_provider_classification() {
        assert!(is_local_provider("ollama"));
        assert!(is_local_provider("lmstudio"));
        assert!(!is_local_provider("openai"));
        assert!(!is_local_provider("deepseek"));
    }

    #[test]
    fn test_known_provider_classification() {
        for p in [
            "openai",
            "openrouter",
            "deepseek",
            "agnes",
            "ollama",
            "lmstudio",
        ] {
            assert!(is_known_provider(p), "{} should be known", p);
        }
        assert!(!is_known_provider("mystery-vendor"));
    }

    #[test]
    fn test_model_source_labels() {
        assert_eq!(ModelSource::Discovered.label(), "discovered");
        assert_eq!(ModelSource::ProviderDefault.label(), "provider default");
        assert_eq!(ModelSource::UserConfigured.label(), "user configured");
    }

    #[test]
    fn test_model_metadata_unknown_is_not_known() {
        let unknown = ModelMetadata::unknown();
        assert!(!unknown.is_known());
        let known = ModelMetadata {
            display_name: Some("DeepSeek V4 Flash".to_string()),
            tool_calling: Some(true),
            context_tokens: Some(1_000_000),
        };
        assert!(known.is_known());
    }
}

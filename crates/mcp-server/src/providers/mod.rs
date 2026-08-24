#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement
mod catalog;
mod models;

pub use catalog::{
    default_base_url, fallback_catalog, is_known_provider, is_local_provider,
    provider_display_name, CatalogModel, ModelMetadata, ModelSource, AGNES_CATALOG,
    DEEPSEEK_CATALOG,
};
pub use models::{
    actionable_model_error, discover_model, discover_models, fetch_models, fetch_models_raw,
    is_auth_failure, pick_default, pick_default_from_discovery, DiscoveredModel, DiscoveryError,
    ModelDiscovery,
};

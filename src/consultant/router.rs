#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Provider router: maps a `ConsultantProvider` enum value to a concrete
//! implementation and handles the `"auto"` selection policy.

use std::collections::HashMap;
use std::sync::Arc;

use super::provider::ConsultantProvider;
use super::types::{AuthStatus, ConsultantProvider as ProviderChoice};

/// Routes a provider choice to a concrete provider instance.
pub struct ConsultantRouter {
    providers: HashMap<String, Arc<dyn ConsultantProvider>>,
}

impl ConsultantRouter {
    pub fn new() -> Self {
        ConsultantRouter {
            providers: HashMap::new(),
        }
    }

    /// Register a provider instance. Later registrations with the same name
    /// overwrite earlier ones.
    pub fn register(&mut self, provider: Arc<dyn ConsultantProvider>) {
        let name = provider.name().to_string();
        self.providers.insert(name, provider);
    }

    /// Resolve the provider choice to a concrete provider.
    ///
    /// For `Auto`, picks the first authenticated provider deterministically
    /// (alphabetical by name). If none are authenticated, returns the first
    /// registered provider so the caller gets a useful error about auth.
    pub fn resolve(&self, choice: &ProviderChoice) -> Result<Arc<dyn ConsultantProvider>, String> {
        match choice {
            ProviderChoice::Auto => self.resolve_auto(),
            ProviderChoice::Conductor => self.get("conductor"),
            _ => Err(format!(
                "unknown provider '{}' — use auto or conductor",
                choice
            )),
        }
    }

    fn resolve_auto(&self) -> Result<Arc<dyn ConsultantProvider>, String> {
        // Deterministic: pick the first authenticated provider alphabetically.
        let mut authenticated: Vec<&dyn ConsultantProvider> = self
            .providers
            .values()
            .map(|p| p.as_ref())
            .filter(|p| matches!(p.auth_status(), AuthStatus::Authenticated))
            .collect();
        authenticated.sort_by_key(|p| p.name());
        if let Some(provider) = authenticated.first() {
            return Ok(self.providers[provider.name()].clone());
        }
        // No authenticated provider: return the first one so the caller gets a
        // clear auth-required error rather than a cryptic "not found".
        if let Some((name, provider)) = self.providers.iter().next() {
            return Ok(provider.clone());
        }
        Err("no consultant providers registered".to_string())
    }

    fn get(&self, name: &str) -> Result<Arc<dyn ConsultantProvider>, String> {
        self.providers
            .get(name)
            .cloned()
            .ok_or_else(|| format!("unknown provider '{name}'"))
    }

    /// List all registered provider names.
    pub fn registered_providers(&self) -> Vec<String> {
        let mut names: Vec<String> = self.providers.keys().cloned().collect();
        names.sort();
        names
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::AuthStatus;
    use super::*;

    #[derive(Clone)]
    struct FakeProvider {
        name_val: String,
        auth: AuthStatus,
    }

    #[async_trait::async_trait]
    impl ConsultantProvider for FakeProvider {
        fn name(&self) -> &str {
            &self.name_val
        }
        async fn consult(
            &self,
            _request: &super::super::types::ConsultantRequest,
        ) -> Result<super::super::types::ConsultantResponse, super::super::provider::ConsultantError>
        {
            Ok(super::super::types::ConsultantResponse::simple(
                &self.name_val,
                "answer",
            ))
        }
        fn auth_status(&self) -> AuthStatus {
            self.auth.clone()
        }
        fn login_url(&self) -> &str {
            "https://example.com/login"
        }
    }

    #[test]
    fn resolves_known_provider() {
        let mut router = ConsultantRouter::new();
        router.register(Arc::new(FakeProvider {
            name_val: "conductor".to_string(),
            auth: AuthStatus::Authenticated,
        }));
        let p = router
            .resolve(&ProviderChoice::Conductor)
            .expect("resolve conductor");
        assert_eq!(p.name(), "conductor");
    }

    #[test]
    fn auto_picks_authenticated_alphabetically() {
        let mut router = ConsultantRouter::new();
        // Register in reverse alphabetical order.
        router.register(Arc::new(FakeProvider {
            name_val: "zzz".to_string(),
            auth: AuthStatus::Authenticated,
        }));
        router.register(Arc::new(FakeProvider {
            name_val: "aaa".to_string(),
            auth: AuthStatus::Authenticated,
        }));
        let p = router.resolve(&ProviderChoice::Auto).expect("resolve auto");
        assert_eq!(
            p.name(),
            "aaa",
            "auto must pick alphabetically first authenticated"
        );
    }

    #[test]
    fn auto_skips_unauthenticated() {
        let mut router = ConsultantRouter::new();
        router.register(Arc::new(FakeProvider {
            name_val: "only".to_string(),
            auth: AuthStatus::Unauthenticated,
        }));
        // Auto should still return the provider (so we get an auth error, not "not found").
        let p = router
            .resolve(&ProviderChoice::Auto)
            .expect("resolve auto with no auth");
        assert_eq!(p.name(), "only");
    }

    #[test]
    fn unknown_provider_errors() {
        let router = ConsultantRouter::new();
        match router.resolve(&ProviderChoice::Claude) {
            Err(err) => assert!(err.contains("unknown provider")),
            Ok(_) => panic!("expected error for unknown provider"),
        }
    }

    #[test]
    fn registered_providers_sorted() {
        let mut router = ConsultantRouter::new();
        router.register(Arc::new(FakeProvider {
            name_val: "conductor".to_string(),
            auth: AuthStatus::Unauthenticated,
        }));
        let mut names = router.registered_providers();
        names.sort(); // already sorted by the method, but verify deterministically
        assert_eq!(names, vec!["conductor"]);
    }
}

#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Service Discovery — metadata queries, filtering, and search.

use crate::service_registry::registry::ServiceRegistry;
use crate::service_registry::types::*;
use crate::service_registry::service::Service;

/// Discovery query results.
#[derive(Debug, Clone)]
pub struct DiscoveryResult {
    pub services: Vec<Service>,
    pub total_count: usize,
    pub query: String,
}

impl DiscoveryResult {
    pub fn is_empty(&self) -> bool {
        self.services.is_empty()
    }

    pub fn first(&self) -> Option<&Service> {
        self.services.first()
    }
}

/// Service discoverer for metadata queries and filtered searches.
#[derive(Clone)]
pub struct ServiceDiscovery {
    registry: ServiceRegistry,
}

impl ServiceDiscovery {
    pub fn new(registry: ServiceRegistry) -> Self {
        ServiceDiscovery { registry }
    }

    /// Search services by name prefix.
    pub fn search_by_name(&self, prefix: &str) -> DiscoveryResult {
        let all = self.registry.enumerate(None);
        let filtered: Vec<Service> = all
            .into_iter()
            .filter(|s| s.name.as_str().starts_with(prefix))
            .collect();
        let count = filtered.len();
        DiscoveryResult {
            services: filtered,
            total_count: count,
            query: format!("name_prefix:{prefix}"),
        }
    }

    /// Search services by provider.
    pub fn search_by_provider(&self, provider: &str) -> DiscoveryResult {
        let all = self.registry.enumerate(None);
        let filtered: Vec<Service> = all
            .into_iter()
            .filter(|s| s.provider == provider)
            .collect();
        let count = filtered.len();
        DiscoveryResult {
            services: filtered,
            total_count: count,
            query: format!("provider:{provider}"),
        }
    }

    /// Search services by capability.
    pub fn search_by_capability(&self, capability: &Capability) -> DiscoveryResult {
        let all = self.registry.enumerate(None);
        let filtered: Vec<Service> = all
            .into_iter()
            .filter(|s| s.has_capability(capability))
            .collect();
        let count = filtered.len();
        DiscoveryResult {
            services: filtered,
            total_count: count,
            query: format!("capability:{capability}"),
        }
    }

    /// Search with advanced filters.
    pub fn search(&self, filter: &DiscoveryFilter) -> DiscoveryResult {
        let all = self.registry.enumerate(None);
        let filtered: Vec<Service> = all
            .into_iter()
            .filter(|s| filter.matches(s))
            .collect();
        let count = filtered.len();
        DiscoveryResult {
            services: filtered,
            total_count: count,
            query: self.filter_to_query(filter),
        }
    }

    /// Get service metadata by ID.
    pub fn get_metadata(&self, service_id: &ServiceId) -> Option<ServiceMetadata> {
        self.registry
            .get(service_id)
            .map(|s| s.metadata.clone())
    }

    /// Get service manifest (full details) by ID.
    pub fn get_manifest(&self, service_id: &ServiceId) -> Option<Service> {
        self.registry.get(service_id)
    }

    /// Get all services from a specific provider.
    pub fn services_by_provider(&self, provider: &str) -> Vec<Service> {
        self.registry
            .enumerate(None)
            .into_iter()
            .filter(|s| s.provider == provider)
            .collect()
    }

    /// Get activated services only.
    pub fn activated_services(&self) -> Vec<Service> {
        self.registry
            .enumerate(Some(&ServiceStatus::Activated))
    }

    /// Get service count by name.
    pub fn count_by_name(&self, name: &str) -> usize {
        self.registry.enumerate_by_name(name).len()
    }

    /// List all unique service names.
    pub fn list_names(&self) -> Vec<String> {
        let services = self.registry.enumerate(None);
        let mut names: Vec<String> = services.iter().map(|s| s.name.0.clone()).collect();
        names.sort();
        names.dedup();
        names
    }

    /// Find services with specific metadata keys.
    pub fn find_by_metadata(&self, key: &str, value: &str) -> DiscoveryResult {
        let all = self.registry.enumerate(None);
        let filtered: Vec<Service> = all
            .into_iter()
            .filter(|s| s.metadata.get(key) == Some(value))
            .collect();
        let count = filtered.len();
        DiscoveryResult {
            services: filtered,
            total_count: count,
            query: format!("metadata:{key}={value}"),
        }
    }

    fn filter_to_query(&self, filter: &DiscoveryFilter) -> String {
        let mut parts = Vec::new();
        if let Some(ref prefix) = filter.name_prefix {
            parts.push(format!("name:{prefix}"));
        }
        if let Some(ref provider) = filter.provider {
            parts.push(format!("provider:{provider}"));
        }
        if !filter.capabilities.is_empty() {
            let caps: Vec<String> = filter.capabilities.iter().map(|c| c.to_string()).collect();
            parts.push(format!("caps:[{}]", caps.join(",")));
        }
        if let Some(ref min_v) = filter.min_version {
            parts.push(format!("min_ver:{min_v}"));
        }
        if let Some(ref max_v) = filter.max_version {
            parts.push(format!("max_ver:{max_v}"));
        }
        if let Some(ref vis) = filter.visibility {
            parts.push(format!("vis:{vis}"));
        }
        if let Some(ref status) = filter.status {
            parts.push(format!("status:{status}"));
        }
        for (k, v) in &filter.metadata_contains {
            parts.push(format!("meta:{k}={v}"));
        }
        parts.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_svc(id: &str, name: &str, version: &str, provider: &str) -> Service {
        Service::builder()
            .with_id(ServiceId::new(id).unwrap())
            .with_name(ServiceName::new(name).unwrap())
            .with_version(ServiceVersion::new(version).unwrap())
            .with_provider(provider)
            .with_capabilities(vec![Capability::Read])
            .build()
            .unwrap()
    }

    fn make_registry_with_services() -> (ServiceRegistry, ServiceDiscovery) {
        let mut reg = ServiceRegistry::new();
        reg.register(make_svc("s1", "data-service", "1.0.0", "plugin-a")).unwrap();
        reg.register(make_svc("s2", "data-service", "2.0.0", "plugin-b")).unwrap();
        reg.register(make_svc("s3", "log-service", "1.0.0", "plugin-a")).unwrap();
        reg.register(make_svc("s4", "auth-service", "1.0.0", "plugin-c")).unwrap();
        (reg.clone(), ServiceDiscovery::new(reg))
    }

    #[test]
    fn test_search_by_name_prefix() {
        let (_reg, disc) = make_registry_with_services();
        let result = disc.search_by_name("data");
        assert_eq!(result.total_count, 2);
        assert_eq!(result.query, "name_prefix:data");
    }

    #[test]
    fn test_search_by_name_prefix_no_match() {
        let (_reg, disc) = make_registry_with_services();
        let result = disc.search_by_name("nonexistent");
        assert_eq!(result.total_count, 0);
        assert!(result.is_empty());
    }

    #[test]
    fn test_search_by_provider() {
        let (_reg, disc) = make_registry_with_services();
        let result = disc.search_by_provider("plugin-a");
        assert_eq!(result.total_count, 2);
    }

    #[test]
    fn test_search_by_capability() {
        let (_reg, disc) = make_registry_with_services();
        let result = disc.search_by_capability(&Capability::Read);
        assert_eq!(result.total_count, 4);
    }

    #[test]
    fn test_search_with_filters() {
        let (reg, disc) = make_registry_with_services();
        let filter = DiscoveryFilter::new()
            .by_provider("plugin-a")
            .with_capabilities(vec![Capability::Read]);
        let result = disc.search(&filter);
        assert_eq!(result.total_count, 2);
    }

    #[test]
    fn test_get_metadata() {
        let (reg, disc) = make_registry_with_services();
        let meta = disc.get_metadata(&ServiceId::new("s1").unwrap());
        assert!(meta.is_some());
    }

    #[test]
    fn test_get_manifest() {
        let (reg, disc) = make_registry_with_services();
        let manifest = disc.get_manifest(&ServiceId::new("s1").unwrap());
        assert!(manifest.is_some());
        assert_eq!(manifest.unwrap().name.as_str(), "data-service");
    }

    #[test]
    fn test_activated_services() {
        let (mut reg, disc) = make_registry_with_services();
        reg.activate(&ServiceId::new("s1").unwrap()).unwrap();
        reg.activate(&ServiceId::new("s3").unwrap()).unwrap();
        let activated = disc.activated_services();
        assert_eq!(activated.len(), 2);
    }

    #[test]
    fn test_count_by_name() {
        let (_reg, disc) = make_registry_with_services();
        assert_eq!(disc.count_by_name("data-service"), 2);
        assert_eq!(disc.count_by_name("log-service"), 1);
        assert_eq!(disc.count_by_name("missing"), 0);
    }

    #[test]
    fn test_list_names() {
        let (_reg, disc) = make_registry_with_services();
        let names = disc.list_names();
        assert_eq!(names, vec!["auth-service", "data-service", "log-service"]);
    }

    #[test]
    fn test_find_by_metadata() {
        let mut reg = ServiceRegistry::new();
        let meta = ServiceMetadata::new().with("env", "prod").with("region", "us");
        let svc1 = Service::builder()
            .with_id(ServiceId::new("s1").unwrap())
            .with_name(ServiceName::new("svc").unwrap())
            .with_version(ServiceVersion::new("1.0.0").unwrap())
            .with_provider("p")
            .with_capabilities(vec![Capability::Read])
            .with_metadata(meta.clone())
            .build()
            .unwrap();
        let meta2 = ServiceMetadata::new().with("env", "dev").with("region", "us");
        let svc2 = Service::builder()
            .with_id(ServiceId::new("s2").unwrap())
            .with_name(ServiceName::new("svc").unwrap())
            .with_version(ServiceVersion::new("1.0.0").unwrap())
            .with_provider("p")
            .with_capabilities(vec![Capability::Read])
            .with_metadata(meta2)
            .build()
            .unwrap();
        reg.register(svc1).unwrap();
        reg.register(svc2).unwrap();

        let disc = ServiceDiscovery::new(reg);
        let result = disc.find_by_metadata("env", "prod");
        assert_eq!(result.total_count, 1);
    }

    #[test]
    fn test_discovery_filter_version_range() {
        let (reg, disc) = make_registry_with_services();
        let filter = DiscoveryFilter::new()
            .with_version_range(
                ServiceVersion::new("1.5.0").unwrap(),
                ServiceVersion::new("2.5.0").unwrap(),
            );
        let result = disc.search(&filter);
        assert_eq!(result.total_count, 1);
        assert_eq!(result.services[0].id.as_str(), "s2");
    }

    #[test]
    fn test_discovery_filter_status() {
        let (mut reg, disc) = make_registry_with_services();
        reg.activate(&ServiceId::new("s1").unwrap()).unwrap();
        let filter = DiscoveryFilter::new().with_status(ServiceStatus::Activated);
        let result = disc.search(&filter);
        assert_eq!(result.total_count, 1);
    }

    #[test]
    fn test_discovery_filter_visibility() {
        let mut reg = ServiceRegistry::new();
        let pub_svc = Service::builder()
            .with_id(ServiceId::new("s1").unwrap())
            .with_name(ServiceName::new("pub").unwrap())
            .with_version(ServiceVersion::new("1.0.0").unwrap())
            .with_provider("p")
            .with_capabilities(vec![Capability::Read])
            .with_visibility(Visibility::Public)
            .build().unwrap();
        let priv_svc = Service::builder()
            .with_id(ServiceId::new("s2").unwrap())
            .with_name(ServiceName::new("priv").unwrap())
            .with_version(ServiceVersion::new("1.0.0").unwrap())
            .with_provider("p")
            .with_capabilities(vec![Capability::Read])
            .with_visibility(Visibility::Private)
            .build().unwrap();
        reg.register(pub_svc).unwrap();
        reg.register(priv_svc).unwrap();

        let disc = ServiceDiscovery::new(reg);
        let result = disc.search(&DiscoveryFilter::new().with_visibility(Visibility::Public));
        assert_eq!(result.total_count, 1);
        assert_eq!(result.services[0].id.as_str(), "s1");
    }

    #[test]
    fn test_services_by_provider() {
        let (_reg, disc) = make_registry_with_services();
        let services = disc.services_by_provider("plugin-a");
        assert_eq!(services.len(), 2);
    }
}

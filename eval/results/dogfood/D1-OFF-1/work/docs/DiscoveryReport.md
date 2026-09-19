# Discovery Report

**P9.4 — Service Registry Foundation**
**Module:** `src/service_registry/discovery.rs`

---

## 1. Overview

The discovery layer provides metadata queries, filtering, and search capabilities over the service registry. It is read-heavy and designed for plugins to explore what services are available without direct inter-plugin communication.

---

## 2. Public API

```rust
// Basic searches
fn search_by_name(&self, prefix: &str) -> DiscoveryResult
fn search_by_provider(&self, provider: &str) -> DiscoveryResult
fn search_by_capability(&self, capability: &Capability) -> DiscoveryResult

// Advanced search with filters
fn search(&self, filter: &DiscoveryFilter) -> DiscoveryResult

// Metadata access
fn get_metadata(&self, service_id: &ServiceId) -> Option<ServiceMetadata>
fn get_manifest(&self, service_id: &ServiceId) -> Option<Service>

// Aggregated views
fn services_by_provider(&self, provider: &str) -> Vec<Service>
fn activated_services(&self) -> Vec<Service>
fn count_by_name(&self, name: &str) -> usize
fn list_names(&self) -> Vec<String>
fn find_by_metadata(&self, key: &str, value: &str) -> DiscoveryResult
```

---

## 3. DiscoveryFilter

```rust
pub struct DiscoveryFilter {
    pub name_prefix: Option<String>,
    pub provider: Option<String>,
    pub capabilities: Vec<Capability>,
    pub min_version: Option<ServiceVersion>,
    pub max_version: Option<ServiceVersion>,
    pub visibility: Option<Visibility>,
    pub status: Option<ServiceStatus>,
    pub metadata_contains: HashMap<String, String>,
}
```

### Builder Methods
```rust
filter.by_name_prefix("data")
      .by_provider("plugin-a")
      .with_capabilities(vec![Capability::Read])
      .with_version_range(min, max)
      .with_visibility(Visibility::Public)
      .with_status(ServiceStatus::Activated)
```

---

## 4. DiscoveryResult

```rust
pub struct DiscoveryResult {
    pub services: Vec<Service>,
    pub total_count: usize,
    pub query: String,  // Human-readable query description
}
```

Methods:
- `is_empty() -> bool`
- `first() -> Option<&Service>`

---

## 5. Search Semantics

### AND Logic
All specified filter criteria must match (AND semantics).

### Capability Matching
When `capabilities` is specified, ALL listed capabilities must be present on the service.

### Version Range
Both min and max are inclusive.

### Metadata Matching
Exact key-value match required.

---

## 6. Test Coverage

**15 tests** covering:
- Name prefix search
- Provider search
- Capability search
- Combined filter search
- Metadata retrieval
- Manifest retrieval
- Activated services filtering
- Count by name
- Name listing (sorted, deduplicated)
- Metadata-based search
- Version range filtering
- Status filtering
- Visibility filtering
- Provider service listing

---

## 7. Performance Characteristics

- **Linear scan**: All searches iterate over registered services
- **No indexing**: Name indexing is maintained by the registry for `enumerate_by_name`
- **Clone-on-return**: Services are cloned for each result (immutable after registration)
- **Filter composition**: Multiple filters are composed in a single pass

---

## 8. Use Cases

1. **Plugin startup**: Discover available services before initializing
2. **Capability discovery**: Find services that provide a specific capability
3. **Health monitoring**: List activated services for status dashboards
4. **Debugging**: Search by metadata to trace service origins
5. **Namespace isolation**: Filter by namespace visibility

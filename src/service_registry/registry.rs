#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Core Service Registry — register, unregister, activate, deactivate, enumerate.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex};

use crate::observability::{CorrelationId, Event, EventBus, EventType};
use crate::service_registry::service::Service;
use crate::service_registry::types::*;

pub(crate) struct RegistryInner {
    pub services: HashMap<ServiceId, Service>,
    pub by_name: HashMap<String, Vec<ServiceId>>,
    pub registration_counter: u64,
    pub event_bus: Option<EventBus>,
}

/// The Service Registry — central coordination layer for inter-plugin communication.
///
/// Plugins MUST NOT keep direct references to each other.
/// All communication flows through this registry.
#[derive(Clone)]
pub struct ServiceRegistry {
    pub inner: Arc<Mutex<RegistryInner>>,
}

impl ServiceRegistry {
    pub fn new() -> Self {
        ServiceRegistry {
            inner: Arc::new(Mutex::new(RegistryInner {
                services: HashMap::new(),
                by_name: HashMap::new(),
                registration_counter: 0,
                event_bus: None,
            })),
        }
    }

    pub fn with_event_bus(event_bus: EventBus) -> Self {
        ServiceRegistry {
            inner: Arc::new(Mutex::new(RegistryInner {
                services: HashMap::new(),
                by_name: HashMap::new(),
                registration_counter: 0,
                event_bus: Some(event_bus),
            })),
        }
    }

    /// Register a service in the registry.
    ///
    /// Returns the registration order assigned to this service.
    pub fn register(&mut self, service: Service) -> Result<u64, RegistryError> {
        let id = service.id.clone();

        if self.inner.lock().unwrap().services.contains_key(&id) {
            return Err(RegistryError::AlreadyRegistered(id));
        }

        let order;
        {
            let mut inner = self.inner.lock().unwrap();
            order = inner.registration_counter;
            inner.registration_counter += 1;
            inner.services.insert(id.clone(), service.clone());
            inner
                .by_name
                .entry(service.name.as_str().to_string())
                .or_default()
                .push(id.clone());
            drop(inner);
        }

        self.emit_event(Event::new(
            EventType::Custom("ServiceRegistered".to_string()),
            CorrelationId::new(),
            "service_registry",
            &format!("Service registered: {id} v{}", service.version),
        ));

        Ok(order)
    }

    /// Unregister a service from the registry.
    pub fn unregister(&mut self, service_id: &ServiceId) -> Result<Service, RegistryError> {
        let mut inner = self.inner.lock().unwrap();
        let service = inner
            .services
            .remove(service_id)
            .ok_or(RegistryError::NotFound(service_id.clone()))?;

        if let Some(ids) = inner.by_name.get_mut(service.name.as_str()) {
            ids.retain(|id| id != service_id);
            if ids.is_empty() {
                inner.by_name.remove(service.name.as_str());
            }
        }
        drop(inner);

        self.emit_event(Event::new(
            EventType::Custom("ServiceUnregistered".to_string()),
            CorrelationId::new(),
            "service_registry",
            &format!("Service unregistered: {service_id}"),
        ));

        Ok(service)
    }

    /// Activate a registered service.
    pub fn activate(&mut self, service_id: &ServiceId) -> Result<(), RegistryError> {
        let mut inner = self.inner.lock().unwrap();
        let service = inner
            .services
            .get_mut(service_id)
            .ok_or(RegistryError::NotFound(service_id.clone()))?;

        match &service.status {
            ServiceStatus::Activated => {
                return Err(RegistryError::AlreadyActivated(service_id.clone()))
            }
            ServiceStatus::Error(_) => return Err(RegistryError::ServiceError(service_id.clone())),
            _ => {}
        }

        service.status = ServiceStatus::Activated;
        drop(inner);

        self.emit_event(Event::new(
            EventType::Custom("ServiceActivated".to_string()),
            CorrelationId::new(),
            "service_registry",
            &format!("Service activated: {service_id}"),
        ));

        Ok(())
    }

    /// Deactivate a service.
    pub fn deactivate(&mut self, service_id: &ServiceId) -> Result<(), RegistryError> {
        let mut inner = self.inner.lock().unwrap();
        let service = inner
            .services
            .get_mut(service_id)
            .ok_or(RegistryError::NotFound(service_id.clone()))?;

        match &service.status {
            ServiceStatus::Deactivated => {
                return Err(RegistryError::AlreadyDeactivated(service_id.clone()))
            }
            _ => {}
        }

        service.status = ServiceStatus::Deactivated;
        drop(inner);

        self.emit_event(Event::new(
            EventType::Custom("ServiceDeactivated".to_string()),
            CorrelationId::new(),
            "service_registry",
            &format!("Service deactivated: {service_id}"),
        ));

        Ok(())
    }

    /// Get a service by ID.
    pub fn get(&self, service_id: &ServiceId) -> Option<Service> {
        self.inner.lock().unwrap().services.get(service_id).cloned()
    }

    /// Enumerate all services matching the given status filter.
    pub fn enumerate(&self, status: Option<&ServiceStatus>) -> Vec<Service> {
        let inner = self.inner.lock().unwrap();
        let mut services: Vec<Service> = inner.services.values().cloned().collect();
        if let Some(s) = status {
            services.retain(|svc| &svc.status == s);
        }
        services
    }

    /// Enumerate all services by name.
    pub fn enumerate_by_name(&self, name: &str) -> Vec<Service> {
        let inner = self.inner.lock().unwrap();
        inner
            .by_name
            .get(name)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| inner.services.get(id).cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get the total number of registered services.
    pub fn count(&self) -> usize {
        self.inner.lock().unwrap().services.len()
    }

    /// Get the current registration counter.
    pub fn registration_count(&self) -> u64 {
        self.inner.lock().unwrap().registration_counter
    }

    /// Check if a service exists.
    pub fn contains(&self, service_id: &ServiceId) -> bool {
        self.inner.lock().unwrap().services.contains_key(service_id)
    }

    fn emit_event(&self, event: Event) {
        if let Some(ref bus) = self.inner.lock().unwrap().event_bus {
            bus.emit(&event);
        }
    }
}

impl Default for ServiceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub enum RegistryError {
    AlreadyRegistered(ServiceId),
    NotFound(ServiceId),
    AlreadyActivated(ServiceId),
    AlreadyDeactivated(ServiceId),
    ServiceError(ServiceId),
    InvalidService(String),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::AlreadyRegistered(id) => {
                write!(f, "Service already registered: {id}")
            }
            RegistryError::NotFound(id) => write!(f, "Service not found: {id}"),
            RegistryError::AlreadyActivated(id) => write!(f, "Service already activated: {id}"),
            RegistryError::AlreadyDeactivated(id) => {
                write!(f, "Service already deactivated: {id}")
            }
            RegistryError::ServiceError(id) => write!(f, "Service in error state: {id}"),
            RegistryError::InvalidService(msg) => write!(f, "Invalid service: {msg}"),
        }
    }
}

impl std::error::Error for RegistryError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_service(id: &str, name: &str, version: &str, provider: &str) -> Service {
        Service::builder()
            .with_id(ServiceId::new(id).unwrap())
            .with_name(ServiceName::new(name).unwrap())
            .with_version(ServiceVersion::new(version).unwrap())
            .with_provider(provider)
            .with_capabilities(vec![Capability::Read])
            .build()
            .unwrap()
    }

    #[test]
    fn test_register_and_get() {
        let mut reg = ServiceRegistry::new();
        let svc = test_service("s1", "test-svc", "1.0.0", "plugin-a");
        let order = reg.register(svc).unwrap();
        assert_eq!(order, 0);
        assert_eq!(reg.count(), 1);
        assert!(reg.contains(&ServiceId::new("s1").unwrap()));
    }

    #[test]
    fn test_register_duplicate_fails() {
        let mut reg = ServiceRegistry::new();
        let svc = test_service("s1", "test", "1.0.0", "p");
        reg.register(svc.clone()).unwrap();
        let result = reg.register(svc);
        assert!(result.is_err());
    }

    #[test]
    fn test_unregister() {
        let mut reg = ServiceRegistry::new();
        let svc = test_service("s1", "test", "1.0.0", "p");
        reg.register(svc.clone()).unwrap();
        assert_eq!(reg.count(), 1);
        let removed = reg.unregister(&ServiceId::new("s1").unwrap()).unwrap();
        assert_eq!(removed.id.as_str(), "s1");
        assert_eq!(reg.count(), 0);
        assert!(!reg.contains(&ServiceId::new("s1").unwrap()));
    }

    #[test]
    fn test_unregister_not_found() {
        let mut reg = ServiceRegistry::new();
        let result = reg.unregister(&ServiceId::new("nonexistent").unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn test_activate() {
        let mut reg = ServiceRegistry::new();
        let svc = test_service("s1", "test", "1.0.0", "p");
        reg.register(svc).unwrap();
        reg.activate(&ServiceId::new("s1").unwrap()).unwrap();
        let svc = reg.get(&ServiceId::new("s1").unwrap()).unwrap();
        assert_eq!(svc.status, ServiceStatus::Activated);
    }

    #[test]
    fn test_activate_already_activated() {
        let mut reg = ServiceRegistry::new();
        let svc = test_service("s1", "test", "1.0.0", "p");
        reg.register(svc).unwrap();
        reg.activate(&ServiceId::new("s1").unwrap()).unwrap();
        let result = reg.activate(&ServiceId::new("s1").unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn test_deactivate() {
        let mut reg = ServiceRegistry::new();
        let svc = test_service("s1", "test", "1.0.0", "p");
        reg.register(svc).unwrap();
        reg.activate(&ServiceId::new("s1").unwrap()).unwrap();
        reg.deactivate(&ServiceId::new("s1").unwrap()).unwrap();
        let svc = reg.get(&ServiceId::new("s1").unwrap()).unwrap();
        assert_eq!(svc.status, ServiceStatus::Deactivated);
    }

    #[test]
    fn test_deactivate_not_activated() {
        let mut reg = ServiceRegistry::new();
        let svc = test_service("s1", "test", "1.0.0", "p");
        reg.register(svc).unwrap();
        reg.deactivate(&ServiceId::new("s1").unwrap()).unwrap();
    }

    #[test]
    fn test_enumerate_all() {
        let mut reg = ServiceRegistry::new();
        reg.register(test_service("s1", "svc-a", "1.0.0", "p"))
            .unwrap();
        reg.register(test_service("s2", "svc-b", "1.0.0", "p"))
            .unwrap();
        let all = reg.enumerate(None);
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_enumerate_by_status() {
        let mut reg = ServiceRegistry::new();
        let s1 = test_service("s1", "svc-a", "1.0.0", "p");
        let s2 = test_service("s2", "svc-b", "1.0.0", "p");
        reg.register(s1).unwrap();
        reg.register(s2).unwrap();
        reg.activate(&ServiceId::new("s1").unwrap()).unwrap();

        let activated = reg.enumerate(Some(&ServiceStatus::Activated));
        assert_eq!(activated.len(), 1);
        assert_eq!(activated[0].id.as_str(), "s1");

        let registered = reg.enumerate(Some(&ServiceStatus::Registered));
        assert_eq!(registered.len(), 1);
        assert_eq!(registered[0].id.as_str(), "s2");
    }

    #[test]
    fn test_enumerate_by_name() {
        let mut reg = ServiceRegistry::new();
        reg.register(test_service("s1", "data", "1.0.0", "p1"))
            .unwrap();
        reg.register(test_service("s2", "data", "1.1.0", "p2"))
            .unwrap();
        reg.register(test_service("s3", "other", "1.0.0", "p1"))
            .unwrap();

        let datas = reg.enumerate_by_name("data");
        assert_eq!(datas.len(), 2);

        let others = reg.enumerate_by_name("other");
        assert_eq!(others.len(), 1);
        assert_eq!(others[0].id.as_str(), "s3");
    }

    #[test]
    fn test_registration_order() {
        let mut reg = ServiceRegistry::new();
        let o1 = reg.register(test_service("s1", "a", "1.0.0", "p")).unwrap();
        let o2 = reg.register(test_service("s2", "b", "1.0.0", "p")).unwrap();
        let o3 = reg.register(test_service("s3", "c", "1.0.0", "p")).unwrap();
        assert_eq!(o1, 0);
        assert_eq!(o2, 1);
        assert_eq!(o3, 2);
    }

    #[test]
    fn test_event_emission() {
        let event_bus = EventBus::new();
        let mut reg = ServiceRegistry::with_event_bus(event_bus.clone());
        let svc = test_service("s1", "test", "1.0.0", "p");
        reg.register(svc).unwrap();
        reg.activate(&ServiceId::new("s1").unwrap()).unwrap();

        let events = event_bus.buffer();
        assert!(events.len() >= 2);
        let types: Vec<&EventType> = events.iter().map(|e| &e.event_type).collect();
        assert!(types
            .iter()
            .any(|t| matches!(t, EventType::Custom(s) if s == "ServiceRegistered")));
        assert!(types
            .iter()
            .any(|t| matches!(t, EventType::Custom(s) if s == "ServiceActivated")));
    }

    #[test]
    fn test_thread_safety() {
        use std::thread;
        let reg = ServiceRegistry::new();
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let mut r = reg.clone();
                thread::spawn(move || {
                    let id = format!("s{i}");
                    let svc = test_service(&id, "concurrent", "1.0.0", "p");
                    r.register(svc).unwrap();
                    r.activate(&ServiceId::new(&id).unwrap()).unwrap();
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(reg.count(), 10);
    }

    #[test]
    fn test_unregister_removes_from_index() {
        let mut reg = ServiceRegistry::new();
        reg.register(test_service("s1", "data", "1.0.0", "p"))
            .unwrap();
        reg.register(test_service("s2", "data", "1.1.0", "p"))
            .unwrap();
        reg.unregister(&ServiceId::new("s1").unwrap()).unwrap();
        let datas = reg.enumerate_by_name("data");
        assert_eq!(datas.len(), 1);
        assert_eq!(datas[0].id.as_str(), "s2");
    }
}

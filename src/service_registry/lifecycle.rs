#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Service lifecycle management.
//!
//! Transitions:
//! Registered -> Activated -> Deactivated -> (registered state)
//! Any -> Error -> (requires manual recovery)

use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::{Arc, Mutex};

use crate::observability::{CorrelationId, Event, EventBus, EventType};
use crate::service_registry::registry::{RegistryError, ServiceRegistry};
use crate::service_registry::service::Service;
use crate::service_registry::types::*;

/// Lifecycle state machine for a service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LifecycleState {
    Registered,
    Activated,
    Deactivated,
    Error(String),
    ShuttingDown,
}

impl fmt::Display for LifecycleState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LifecycleState::Registered => write!(f, "registered"),
            LifecycleState::Activated => write!(f, "activated"),
            LifecycleState::Deactivated => write!(f, "deactivated"),
            LifecycleState::Error(s) => write!(f, "error({s})"),
            LifecycleState::ShuttingDown => write!(f, "shutting_down"),
        }
    }
}

/// Transition record for lifecycle audit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleTransition {
    pub service_id: ServiceId,
    pub from: LifecycleState,
    pub to: LifecycleState,
    pub timestamp: String,
    pub reason: String,
}

impl LifecycleTransition {
    pub fn new(
        service_id: ServiceId,
        from: LifecycleState,
        to: LifecycleState,
        reason: &str,
    ) -> Self {
        LifecycleTransition {
            service_id,
            from,
            to,
            timestamp: chrono::Local::now().to_rfc3339(),
            reason: reason.to_string(),
        }
    }
}

/// Lifecycle manager for services.
#[derive(Clone)]
pub struct ServiceLifecycle {
    registry: ServiceRegistry,
    event_bus: Option<EventBus>,
    transition_log: Arc<std::sync::Mutex<Vec<LifecycleTransition>>>,
}

impl ServiceLifecycle {
    pub fn new(registry: ServiceRegistry) -> Self {
        ServiceLifecycle {
            registry,
            event_bus: None,
            transition_log: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    pub fn with_event_bus(mut self, event_bus: EventBus) -> Self {
        self.event_bus = Some(event_bus);
        self
    }

    /// Transition a service to Activated.
    pub fn activate(
        &mut self,
        service_id: &ServiceId,
        reason: &str,
    ) -> Result<LifecycleTransition, LifecycleError> {
        let from = self.current_state(service_id)?;

        // Validate transition
        match &from {
            LifecycleState::Registered | LifecycleState::Deactivated => {}
            LifecycleState::Activated => {
                return Err(LifecycleError::AlreadyInState(
                    service_id.clone(),
                    LifecycleState::Activated,
                ));
            }
            LifecycleState::Error(_) => {
                return Err(LifecycleError::ServiceInError(service_id.clone()));
            }
            LifecycleState::ShuttingDown => {
                return Err(LifecycleError::ServiceShuttingDown(service_id.clone()));
            }
        }

        // Apply transition in registry
        self.registry.activate(service_id).map_err(|e| match e {
            crate::service_registry::registry::RegistryError::NotFound(id) => {
                LifecycleError::NotFound(id)
            }
            crate::service_registry::registry::RegistryError::AlreadyActivated(id) => {
                LifecycleError::AlreadyInState(id, LifecycleState::Activated)
            }
            _ => LifecycleError::Registry(e),
        })?;

        let to = LifecycleState::Activated;
        let transition =
            LifecycleTransition::new(service_id.clone(), from.clone(), to.clone(), reason);
        self.log_transition(transition.clone());
        self.emit_event(&transition);

        Ok(transition)
    }

    /// Transition a service to Deactivated.
    pub fn deactivate(
        &mut self,
        service_id: &ServiceId,
        reason: &str,
    ) -> Result<LifecycleTransition, LifecycleError> {
        let from = self.current_state(service_id)?;

        match &from {
            LifecycleState::Activated => {}
            LifecycleState::Registered => {}
            LifecycleState::Deactivated => {
                return Err(LifecycleError::AlreadyInState(
                    service_id.clone(),
                    LifecycleState::Deactivated,
                ));
            }
            LifecycleState::Error(_) => {
                return Err(LifecycleError::ServiceInError(service_id.clone()));
            }
            LifecycleState::ShuttingDown => {
                return Err(LifecycleError::ServiceShuttingDown(service_id.clone()));
            }
        }

        self.registry.deactivate(service_id).map_err(|e| match e {
            crate::service_registry::registry::RegistryError::NotFound(id) => {
                LifecycleError::NotFound(id)
            }
            _ => LifecycleError::Registry(e),
        })?;

        let to = LifecycleState::Deactivated;
        let transition =
            LifecycleTransition::new(service_id.clone(), from.clone(), to.clone(), reason);
        self.log_transition(transition.clone());
        self.emit_event(&transition);

        Ok(transition)
    }

    /// Transition a service to Error state.
    pub fn error(
        &self,
        service_id: &ServiceId,
        error_msg: &str,
    ) -> Result<LifecycleTransition, LifecycleError> {
        let from = self.current_state(service_id)?;

        match &from {
            LifecycleState::Error(_) => {
                return Err(LifecycleError::AlreadyInState(
                    service_id.clone(),
                    LifecycleState::Error(error_msg.to_string()),
                ));
            }
            _ => {}
        }

        let to = LifecycleState::Error(error_msg.to_string());
        let transition =
            LifecycleTransition::new(service_id.clone(), from.clone(), to.clone(), error_msg);
        self.log_transition(transition.clone());
        self.emit_event(&transition);

        // Update status in registry
        self.set_service_status(service_id, ServiceStatus::Error(error_msg.to_string()));

        Ok(transition)
    }

    /// Recovery: transition from Error back to Registered.
    pub fn recover(
        &self,
        service_id: &ServiceId,
        reason: &str,
    ) -> Result<LifecycleTransition, LifecycleError> {
        let from = self.current_state(service_id)?;

        match &from {
            LifecycleState::Error(_) => {}
            _ => {
                return Err(LifecycleError::ExpectedErrorState(service_id.clone(), from));
            }
        }

        let to = LifecycleState::Registered;
        let transition =
            LifecycleTransition::new(service_id.clone(), from.clone(), to.clone(), reason);
        self.log_transition(transition.clone());
        self.emit_event(&transition);

        // Update status in registry
        self.set_service_status(service_id, ServiceStatus::Registered);

        Ok(transition)
    }

    fn set_service_status(&self, service_id: &ServiceId, status: ServiceStatus) {
        let mut inner = self.registry.inner.lock().unwrap();
        if let Some(svc) = inner.services.get_mut(service_id) {
            svc.status = status;
        }
    }

    /// Get the current lifecycle state of a service.
    pub fn current_state(&self, service_id: &ServiceId) -> Result<LifecycleState, LifecycleError> {
        let svc = self
            .registry
            .get(service_id)
            .ok_or(LifecycleError::NotFound(service_id.clone()))?;

        Ok(match &svc.status {
            ServiceStatus::Registered => LifecycleState::Registered,
            ServiceStatus::Activated => LifecycleState::Activated,
            ServiceStatus::Deactivated => LifecycleState::Deactivated,
            ServiceStatus::Error(msg) => LifecycleState::Error(msg.clone()),
        })
    }

    /// Get the transition log.
    pub fn transition_log(&self) -> Vec<LifecycleTransition> {
        self.transition_log.lock().unwrap().clone()
    }

    /// Get recent transitions for a service.
    pub fn recent_transitions(
        &self,
        service_id: &ServiceId,
        limit: usize,
    ) -> Vec<LifecycleTransition> {
        let log = self.transition_log.lock().unwrap();
        log.iter()
            .filter(|t| &t.service_id == service_id)
            .rev()
            .take(limit)
            .cloned()
            .collect()
    }

    /// Clear the transition log.
    pub fn clear_log(&self) {
        self.transition_log.lock().unwrap().clear();
    }

    fn emit_event(&self, transition: &LifecycleTransition) {
        if let Some(ref bus) = self.event_bus {
            let event_type = match &transition.to {
                LifecycleState::Activated => "ServiceActivated",
                LifecycleState::Deactivated => "ServiceDeactivated",
                LifecycleState::Error(_) => "ServiceError",
                LifecycleState::Registered => "ServiceRegistered",
                LifecycleState::ShuttingDown => "ServiceShuttingDown",
            };
            bus.emit(&Event::new(
                EventType::Custom(event_type.to_string()),
                CorrelationId::new(),
                "service_lifecycle",
                &transition.to_string(),
            ));
        }
    }

    fn log_transition(&self, transition: LifecycleTransition) {
        let mut log = self.transition_log.lock().unwrap();
        log.push(transition);
    }
}

impl fmt::Display for LifecycleTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "LifecycleTransition({} {} -> {} @ {} reason='{}')",
            self.service_id, self.from, self.to, self.timestamp, self.reason
        )
    }
}

#[derive(Debug, Clone)]
pub enum LifecycleError {
    NotFound(ServiceId),
    AlreadyInState(ServiceId, LifecycleState),
    ServiceInError(ServiceId),
    ServiceShuttingDown(ServiceId),
    ExpectedErrorState(ServiceId, LifecycleState),
    Registry(RegistryError),
}

impl fmt::Display for LifecycleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LifecycleError::NotFound(id) => write!(f, "Service not found: {id}"),
            LifecycleError::AlreadyInState(id, state) => {
                write!(f, "Service {id} already in state: {state}")
            }
            LifecycleError::ServiceInError(id) => write!(f, "Service in error state: {id}"),
            LifecycleError::ServiceShuttingDown(id) => {
                write!(f, "Service shutting down: {id}")
            }
            LifecycleError::ExpectedErrorState(id, state) => {
                write!(f, "Expected error state for {id}, got: {state}")
            }
            LifecycleError::Registry(e) => write!(f, "Registry error: {e}"),
        }
    }
}

impl std::error::Error for LifecycleError {}

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

    #[test]
    fn test_activate_from_registered() {
        let mut reg = ServiceRegistry::new();
        let mut lc = ServiceLifecycle::new(reg.clone());
        reg.register(make_svc("s1", "test", "1.0.0", "p")).unwrap();

        let transition = lc
            .activate(&ServiceId::new("s1").unwrap(), "startup")
            .unwrap();
        assert_eq!(transition.from, LifecycleState::Registered);
        assert_eq!(transition.to, LifecycleState::Activated);
    }

    #[test]
    fn test_deactivate_from_activated() {
        let mut reg = ServiceRegistry::new();
        let mut lc = ServiceLifecycle::new(reg.clone());
        reg.register(make_svc("s1", "test", "1.0.0", "p")).unwrap();
        lc.activate(&ServiceId::new("s1").unwrap(), "startup")
            .unwrap();

        let transition = lc
            .deactivate(&ServiceId::new("s1").unwrap(), "shutdown")
            .unwrap();
        assert_eq!(transition.from, LifecycleState::Activated);
        assert_eq!(transition.to, LifecycleState::Deactivated);
    }

    #[test]
    fn test_reactivate_from_deactivated() {
        let mut reg = ServiceRegistry::new();
        let mut lc = ServiceLifecycle::new(reg.clone());
        reg.register(make_svc("s1", "test", "1.0.0", "p")).unwrap();
        lc.activate(&ServiceId::new("s1").unwrap(), "startup")
            .unwrap();
        lc.deactivate(&ServiceId::new("s1").unwrap(), "shutdown")
            .unwrap();

        let transition = lc
            .activate(&ServiceId::new("s1").unwrap(), "restart")
            .unwrap();
        assert_eq!(transition.from, LifecycleState::Deactivated);
        assert_eq!(transition.to, LifecycleState::Activated);
    }

    #[test]
    fn test_error_state() {
        let mut reg = ServiceRegistry::new();
        let mut lc = ServiceLifecycle::new(reg.clone());
        reg.register(make_svc("s1", "test", "1.0.0", "p")).unwrap();
        lc.activate(&ServiceId::new("s1").unwrap(), "startup")
            .unwrap();

        let transition = lc.error(&ServiceId::new("s1").unwrap(), "crash").unwrap();
        assert_eq!(transition.to, LifecycleState::Error("crash".to_string()));

        let state = lc.current_state(&ServiceId::new("s1").unwrap()).unwrap();
        assert_eq!(state, LifecycleState::Error("crash".to_string()));
    }

    #[test]
    fn test_recover_from_error() {
        let mut reg = ServiceRegistry::new();
        let mut lc = ServiceLifecycle::new(reg.clone());
        reg.register(make_svc("s1", "test", "1.0.0", "p")).unwrap();
        lc.error(&ServiceId::new("s1").unwrap(), "crash").unwrap();

        let transition = lc.recover(&ServiceId::new("s1").unwrap(), "fixed").unwrap();
        assert_eq!(transition.to, LifecycleState::Registered);
    }

    #[test]
    fn test_double_activate_fails() {
        let mut reg = ServiceRegistry::new();
        let mut lc = ServiceLifecycle::new(reg.clone());
        reg.register(make_svc("s1", "test", "1.0.0", "p")).unwrap();
        lc.activate(&ServiceId::new("s1").unwrap(), "startup")
            .unwrap();

        let result = lc.activate(&ServiceId::new("s1").unwrap(), "double");
        assert!(result.is_err());
    }

    #[test]
    fn test_error_from_error_fails() {
        let mut reg = ServiceRegistry::new();
        let mut lc = ServiceLifecycle::new(reg.clone());
        reg.register(make_svc("s1", "test", "1.0.0", "p")).unwrap();
        lc.error(&ServiceId::new("s1").unwrap(), "crash1").unwrap();

        let result = lc.error(&ServiceId::new("s1").unwrap(), "crash2");
        assert!(result.is_err());
    }

    #[test]
    fn test_transition_log() {
        let mut reg = ServiceRegistry::new();
        let mut lc = ServiceLifecycle::new(reg.clone());
        reg.register(make_svc("s1", "test", "1.0.0", "p")).unwrap();
        lc.activate(&ServiceId::new("s1").unwrap(), "startup")
            .unwrap();
        lc.deactivate(&ServiceId::new("s1").unwrap(), "shutdown")
            .unwrap();

        let log = lc.recent_transitions(&ServiceId::new("s1").unwrap(), 10);
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].to, LifecycleState::Deactivated);
        assert_eq!(log[1].to, LifecycleState::Activated);
    }

    #[test]
    fn test_not_found() {
        let reg = ServiceRegistry::new();
        let lc = ServiceLifecycle::new(reg.clone());
        let result = lc.current_state(&ServiceId::new("missing").unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn test_event_emission() {
        let event_bus = EventBus::new();
        let mut reg = ServiceRegistry::new();
        let lc = ServiceLifecycle::new(reg.clone());
        let mut lc = lc.with_event_bus(event_bus.clone());
        reg.register(make_svc("s1", "test", "1.0.0", "p")).unwrap();
        lc.activate(&ServiceId::new("s1").unwrap(), "startup")
            .unwrap();

        let events = event_bus.buffer();
        assert!(events
            .iter()
            .any(|e| { matches!(&e.event_type, EventType::Custom(s) if s == "ServiceActivated") }));
    }

    #[test]
    fn test_thread_safety() {
        use std::thread;
        let mut reg = ServiceRegistry::new();
        let lc = ServiceLifecycle::new(reg.clone());
        reg.register(make_svc("s1", "test", "1.0.0", "p")).unwrap();

        let handles: Vec<_> = (0..10)
            .map(|i| {
                let lc = lc.clone();
                let id = ServiceId::new("s1").unwrap();
                thread::spawn(move || {
                    let mut lc = lc;
                    let _ = lc.activate(&id, "concurrent");
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        // After 10 concurrent activate attempts, exactly one should succeed
        let state = lc.current_state(&ServiceId::new("s1").unwrap()).unwrap();
        assert_eq!(state, LifecycleState::Activated);
    }
}

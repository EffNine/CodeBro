//! Preference Events — observable change notifications.
//!
//! All external consumers (TUI, tools, platforms) interact with the engine
//! through this event channel. No direct storage access is permitted.

use crate::preference_engine::schema::{Preference, PreferenceId};
use serde::{Deserialize, Serialize};

/// The timestamp associated with an event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventTimestamp(pub String);

impl EventTimestamp {
    pub fn now() -> Self {
        EventTimestamp(chrono::Utc::now().to_rfc3339())
    }
}

/// Preference engine events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PreferenceEvent {
    PreferenceCreated {
        id: PreferenceId,
        key: String,
        category: String,
        timestamp: EventTimestamp,
    },
    PreferenceUpdated {
        id: PreferenceId,
        key: String,
        new_value: String,
        timestamp: EventTimestamp,
    },
    PreferenceDeleted {
        id: PreferenceId,
        key: String,
        timestamp: EventTimestamp,
    },
    PreferenceImported {
        count: usize,
        timestamp: EventTimestamp,
    },
    PreferenceExported {
        count: usize,
        timestamp: EventTimestamp,
    },
    PreferenceReset {
        count: usize,
        timestamp: EventTimestamp,
    },
}

impl PreferenceEvent {
    pub fn kind(&self) -> &str {
        match self {
            PreferenceEvent::PreferenceCreated { .. } => "created",
            PreferenceEvent::PreferenceUpdated { .. } => "updated",
            PreferenceEvent::PreferenceDeleted { .. } => "deleted",
            PreferenceEvent::PreferenceImported { .. } => "imported",
            PreferenceEvent::PreferenceExported { .. } => "exported",
            PreferenceEvent::PreferenceReset { .. } => "reset",
        }
    }

    pub fn timestamp(&self) -> &str {
        match self {
            PreferenceEvent::PreferenceCreated { timestamp, .. } => &timestamp.0,
            PreferenceEvent::PreferenceUpdated { timestamp, .. } => &timestamp.0,
            PreferenceEvent::PreferenceDeleted { timestamp, .. } => &timestamp.0,
            PreferenceEvent::PreferenceImported { timestamp, .. } => &timestamp.0,
            PreferenceEvent::PreferenceExported { timestamp, .. } => &timestamp.0,
            PreferenceEvent::PreferenceReset { timestamp, .. } => &timestamp.0,
        }
    }
}

/// An in-memory event log.
#[derive(Debug, Default)]
pub struct EventLog {
    events: Vec<PreferenceEvent>,
    max_size: usize,
}

impl EventLog {
    pub fn new(max_size: usize) -> Self {
        EventLog {
            events: Vec::new(),
            max_size,
        }
    }

    pub fn record(&mut self, event: PreferenceEvent) {
        self.events.push(event);
        if self.events.len() > self.max_size {
            self.events.drain(..self.events.len() - self.max_size);
        }
    }

    pub fn events(&self) -> &[PreferenceEvent] {
        &self.events
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn clear(&mut self) {
        self.events.clear();
    }

    pub fn recent(&self, n: usize) -> &[PreferenceEvent] {
        let start = self.events.len().saturating_sub(n);
        &self.events[start..]
    }
}

/// A subscriber receives preference events.
pub trait PreferenceSubscriber: Send + Sync {
    fn on_event(&self, event: &PreferenceEvent);
}

/// A simple in-memory subscriber that stores events for inspection in tests.
#[derive(Debug, Default, Clone)]
pub struct TestSubscriber {
    events: std::sync::Arc<std::sync::Mutex<Vec<PreferenceEvent>>>,
}

impl TestSubscriber {
    pub fn new() -> Self {
        TestSubscriber {
            events: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    pub fn events(&self) -> Vec<PreferenceEvent> {
        self.events.lock().unwrap().clone()
    }
}

impl PreferenceSubscriber for TestSubscriber {
    fn on_event(&self, event: &PreferenceEvent) {
        self.events.lock().unwrap().push(event.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_kind() {
        let e = PreferenceEvent::PreferenceCreated {
            id: PreferenceId::new(),
            key: "model".to_string(),
            category: "model".to_string(),
            timestamp: EventTimestamp::now(),
        };
        assert_eq!(e.kind(), "created");
    }

    #[test]
    fn test_event_timestamp() {
        let ts = EventTimestamp::now();
        assert!(!ts.0.is_empty());
    }

    #[test]
    fn test_event_log_records() {
        let mut log = EventLog::new(100);
        log.record(PreferenceEvent::PreferenceCreated {
            id: PreferenceId::new(),
            key: "model".to_string(),
            category: "model".to_string(),
            timestamp: EventTimestamp::now(),
        });
        assert_eq!(log.len(), 1);
        assert!(!log.is_empty());
    }

    #[test]
    fn test_event_log_max_size() {
        let mut log = EventLog::new(5);
        for i in 0..10 {
            log.record(PreferenceEvent::PreferenceCreated {
                id: PreferenceId::new(),
                key: format!("key_{}", i),
                category: "model".to_string(),
                timestamp: EventTimestamp::now(),
            });
        }
        assert_eq!(log.len(), 5);
    }

    #[test]
    fn test_event_log_recent() {
        let mut log = EventLog::new(100);
        for i in 0..10 {
            log.record(PreferenceEvent::PreferenceCreated {
                id: PreferenceId::new(),
                key: format!("key_{}", i),
                category: "model".to_string(),
                timestamp: EventTimestamp::now(),
            });
        }
        let recent = log.recent(3);
        assert_eq!(recent.len(), 3);
    }

    #[test]
    fn test_test_subscriber() {
        let sub = TestSubscriber::new();
        sub.on_event(&PreferenceEvent::PreferenceCreated {
            id: PreferenceId::new(),
            key: "test".to_string(),
            category: "model".to_string(),
            timestamp: EventTimestamp::now(),
        });
        let events = sub.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind(), "created");
    }

    #[test]
    fn test_all_event_kinds() {
        let created = PreferenceEvent::PreferenceCreated {
            id: PreferenceId::new(),
            key: "k".to_string(),
            category: "c".to_string(),
            timestamp: EventTimestamp::now(),
        };
        let updated = PreferenceEvent::PreferenceUpdated {
            id: PreferenceId::new(),
            key: "k".to_string(),
            new_value: "v".to_string(),
            timestamp: EventTimestamp::now(),
        };
        let deleted = PreferenceEvent::PreferenceDeleted {
            id: PreferenceId::new(),
            key: "k".to_string(),
            timestamp: EventTimestamp::now(),
        };
        let imported = PreferenceEvent::PreferenceImported {
            count: 5,
            timestamp: EventTimestamp::now(),
        };
        let exported = PreferenceEvent::PreferenceExported {
            count: 5,
            timestamp: EventTimestamp::now(),
        };
        let reset = PreferenceEvent::PreferenceReset {
            count: 5,
            timestamp: EventTimestamp::now(),
        };

        assert_eq!(created.kind(), "created");
        assert_eq!(updated.kind(), "updated");
        assert_eq!(deleted.kind(), "deleted");
        assert_eq!(imported.kind(), "imported");
        assert_eq!(exported.kind(), "exported");
        assert_eq!(reset.kind(), "reset");
    }
}

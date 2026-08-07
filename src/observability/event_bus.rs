#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Event Bus — pub/sub for structured observability events.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::observability::types::{CorrelationId, Event, EventType};

const MAX_BUFFERED: usize = 10_000;

pub type EventObserver = Arc<dyn Fn(&Event) + Send + Sync>;

struct EventBusInner {
    observers: Vec<EventObserver>,
    buffer: VecDeque<Event>,
}

#[derive(Clone)]
pub struct EventBus {
    inner: Arc<Mutex<EventBusInner>>,
}

impl EventBus {
    pub fn new() -> Self {
        EventBus {
            inner: Arc::new(Mutex::new(EventBusInner {
                observers: Vec::new(),
                buffer: VecDeque::new(),
            })),
        }
    }

    pub fn subscribe(&self, observer: EventObserver) {
        let mut inner = self.inner.lock().unwrap();
        inner.observers.push(observer);
    }

    pub fn emit(&self, event: &Event) {
        let mut inner = self.inner.lock().unwrap();
        inner.buffer.push_back(event.clone());
        if inner.buffer.len() > MAX_BUFFERED {
            inner.buffer.pop_front();
        }
        for observer in &inner.observers {
            observer(event);
        }
    }

    pub fn buffer(&self) -> Vec<Event> {
        let inner = self.inner.lock().unwrap();
        inner.buffer.iter().cloned().collect()
    }

    pub fn buffer_len(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        inner.buffer.len()
    }

    pub fn observer_count(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        inner.observers.len()
    }

    pub fn clear_buffer(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.buffer.clear();
    }

    pub fn events_by_type(&self, event_type: &EventType) -> Vec<Event> {
        let inner = self.inner.lock().unwrap();
        inner
            .buffer
            .iter()
            .filter(|e| &e.event_type == event_type)
            .cloned()
            .collect()
    }

    pub fn events_by_correlation(&self, correlation_id: &CorrelationId) -> Vec<Event> {
        let inner = self.inner.lock().unwrap();
        inner
            .buffer
            .iter()
            .filter(|e| &e.correlation_id == correlation_id)
            .cloned()
            .collect()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn test_emit_and_observe() {
        let bus = EventBus::new();
        let count = Arc::new(AtomicUsize::new(0));
        let count_clone = count.clone();
        bus.subscribe(Arc::new(move |_event| {
            count_clone.fetch_add(1, Ordering::Relaxed);
        }));
        let event = Event::new(
            EventType::IntentResolved,
            CorrelationId::new(),
            "test",
            "test event",
        );
        bus.emit(&event);
        assert_eq!(count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_multiple_observers() {
        let bus = EventBus::new();
        let c1 = Arc::new(AtomicUsize::new(0));
        let c2 = Arc::new(AtomicUsize::new(0));
        let c1_clone = c1.clone();
        let c2_clone = c2.clone();
        bus.subscribe(Arc::new(move |_| {
            c1_clone.fetch_add(1, Ordering::Relaxed);
        }));
        bus.subscribe(Arc::new(move |_| {
            c2_clone.fetch_add(1, Ordering::Relaxed);
        }));
        let event = Event::new(
            EventType::PipelineCompleted,
            CorrelationId::new(),
            "test",
            "test",
        );
        bus.emit(&event);
        assert_eq!(c1.load(Ordering::Relaxed), 1);
        assert_eq!(c2.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_buffer_stores_events() {
        let bus = EventBus::new();
        let corr = CorrelationId::new();
        bus.emit(&Event::new(
            EventType::IntentResolved,
            corr.clone(),
            "test",
            "e1",
        ));
        bus.emit(&Event::new(
            EventType::WorkflowCreated,
            corr.clone(),
            "test",
            "e2",
        ));
        assert_eq!(bus.buffer_len(), 2);
        let intents = bus.events_by_type(&EventType::IntentResolved);
        assert_eq!(intents.len(), 1);
        let workflows = bus.events_by_type(&EventType::WorkflowCreated);
        assert_eq!(workflows.len(), 1);
    }

    #[test]
    fn test_buffer_bounded() {
        let bus = EventBus::new();
        let corr = CorrelationId::new();
        for i in 0..=MAX_BUFFERED + 100 {
            bus.emit(&Event::new(
                EventType::Custom(format!("ev_{i}")),
                corr.clone(),
                "test",
                "x",
            ));
        }
        assert!(bus.buffer_len() <= MAX_BUFFERED);
    }

    #[test]
    fn test_clear_buffer() {
        let bus = EventBus::new();
        bus.emit(&Event::new(
            EventType::Error,
            CorrelationId::new(),
            "test",
            "err",
        ));
        assert_eq!(bus.buffer_len(), 1);
        bus.clear_buffer();
        assert_eq!(bus.buffer_len(), 0);
    }

    #[test]
    fn test_clone_shares_state() {
        let bus1 = EventBus::new();
        let bus2 = bus1.clone();
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        bus1.subscribe(Arc::new(move |_| {
            c.fetch_add(1, Ordering::Relaxed);
        }));
        bus2.emit(&Event::new(
            EventType::ToolExecuted,
            CorrelationId::new(),
            "test",
            "x",
        ));
        assert_eq!(count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_thread_safety() {
        use std::thread;
        let bus = EventBus::new();
        let corr = CorrelationId::new();
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let b = bus.clone();
                let c = corr.clone();
                thread::spawn(move || {
                    for j in 0..100 {
                        b.emit(&Event::new(
                            EventType::Custom(format!("ev_{}_{}", i, j)),
                            c.clone(),
                            "test",
                            "x",
                        ));
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(bus.buffer_len(), 1000);
    }
}

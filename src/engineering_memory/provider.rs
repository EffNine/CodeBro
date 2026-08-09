//! Trait for subsystems that provide engineering memory snapshots.
//!
//! Future Context Assembly and other consumers depend on this trait rather
//! than the concrete `EngineeringMemoryRuntime`.

use super::resolver::EngineeringMemoryResolver;
use super::types::EngineeringMemoryEntry;
use crate::engineering_context::memory::EngineeringMemoryContext;

/// Provider trait for engineering memory.
pub trait EngineeringMemoryProvider {
    /// Returns the provider name for diagnostics.
    fn provider_name(&self) -> &str;

    /// Returns a snapshot of the current resolved engineering memory context.
    fn snapshot(&self) -> EngineeringMemoryContext;

    /// Resolve memory entries for a specific task query.
    ///
    /// Uses the provider's internal resolver and persistent entries.
    fn resolve_for_task(
        &self,
        task_keywords: &[String],
        active_file_tags: &[String],
    ) -> EngineeringMemoryContext;

    /// Returns the raw entry count in the store.
    fn entry_count(&self) -> usize;
}

/// A stub provider that returns an empty context — useful for testing.
#[derive(Debug, Clone)]
pub struct EmptyEngineeringMemoryProvider {
    name: String,
}

impl EmptyEngineeringMemoryProvider {
    pub fn new(name: impl Into<String>) -> Self {
        EmptyEngineeringMemoryProvider { name: name.into() }
    }
}

impl EngineeringMemoryProvider for EmptyEngineeringMemoryProvider {
    fn provider_name(&self) -> &str {
        &self.name
    }

    fn snapshot(&self) -> EngineeringMemoryContext {
        EngineeringMemoryContext::new()
    }

    fn resolve_for_task(
        &self,
        _task_keywords: &[String],
        _active_file_tags: &[String],
    ) -> EngineeringMemoryContext {
        EngineeringMemoryContext::new()
    }

    fn entry_count(&self) -> usize {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_provider() {
        let provider = EmptyEngineeringMemoryProvider::new("test");
        assert_eq!(provider.provider_name(), "test");
        assert!(provider.snapshot().is_empty());
        assert_eq!(provider.entry_count(), 0);
    }

    #[test]
    fn test_empty_provider_resolve() {
        let provider = EmptyEngineeringMemoryProvider::new("test");
        let result = provider.resolve_for_task(&["auth".to_string()], &["backend".to_string()]);
        assert!(result.is_empty());
    }
}

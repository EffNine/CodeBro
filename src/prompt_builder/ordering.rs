//! Deterministic section ordering for prompt compilation.
//!
//! The ordering module defines the canonical section sequence used by
//! the compiler. It is stable, exhaustive, and independent of template
//! selection — templates select a subset in this canonical order.

use super::template::SectionKey;
use serde::{Deserialize, Serialize};

/// Canonical ordering of all possible prompt sections.
///
/// The compiler always iterates sections in this order. Templates
/// select a subset but never reorder.
pub const CANONICAL_ORDER: &[SectionKey] = &[
    SectionKey::SystemIdentity,
    SectionKey::ProjectIdentity,
    SectionKey::CurrentTask,
    SectionKey::EngineeringObjective,
    SectionKey::EngineeringConstraints,
    SectionKey::RelevantContext,
    SectionKey::StructuredMachineFacts,
    SectionKey::EngineeringMemory,
    SectionKey::ArchitectureDecisions,
    SectionKey::WorkspaceFacts,
    SectionKey::ActiveFiles,
    SectionKey::UserRequest,
    SectionKey::ResponseInstructions,
];

/// A deterministic ordering configuration.
///
/// Wraps a subset of `CANONICAL_ORDER` for a specific template or
/// custom compilation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptOrdering {
    pub keys: Vec<SectionKey>,
}

impl PromptOrdering {
    pub fn from_template(template: super::template::PromptTemplate) -> Self {
        PromptOrdering {
            keys: template.section_order().to_vec(),
        }
    }

    pub fn from_keys(keys: Vec<SectionKey>) -> Self {
        let mut unique = Vec::new();
        for key in keys {
            if !unique.contains(&key) {
                unique.push(key);
            }
        }
        PromptOrdering { keys: unique }
    }

    /// Returns the ordering as a zero-based index map: section_key → position.
    pub fn index_map(&self) -> std::collections::BTreeMap<SectionKey, usize> {
        let mut map = std::collections::BTreeMap::new();
        for (i, key) in self.keys.iter().enumerate() {
            map.insert(*key, i);
        }
        map
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn contains(&self, key: SectionKey) -> bool {
        self.keys.contains(&key)
    }
}

impl Default for PromptOrdering {
    fn default() -> Self {
        PromptOrdering {
            keys: CANONICAL_ORDER.to_vec(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt_builder::template::PromptTemplate;

    #[test]
    fn test_canonical_order_is_complete() {
        assert_eq!(CANONICAL_ORDER.len(), 13);
        assert_eq!(CANONICAL_ORDER[0], SectionKey::SystemIdentity);
        assert_eq!(
            CANONICAL_ORDER[CANONICAL_ORDER.len() - 1],
            SectionKey::ResponseInstructions
        );
    }

    #[test]
    fn test_canonical_order_has_no_duplicates() {
        let mut seen = std::collections::BTreeSet::new();
        for key in CANONICAL_ORDER {
            assert!(
                seen.insert(*key),
                "Duplicate key in canonical order: {:?}",
                key
            );
        }
    }

    #[test]
    fn test_ordering_from_template() {
        let ordering = PromptOrdering::from_template(PromptTemplate::Engineering);
        assert!(ordering.contains(SectionKey::SystemIdentity));
        assert!(ordering.contains(SectionKey::ResponseInstructions));
        assert!(!ordering.is_empty());
    }

    #[test]
    fn test_ordering_from_keys_deduplicates() {
        let ordering = PromptOrdering::from_keys(vec![
            SectionKey::SystemIdentity,
            SectionKey::SystemIdentity,
            SectionKey::UserRequest,
        ]);
        assert_eq!(ordering.len(), 2);
        assert_eq!(ordering.keys[0], SectionKey::SystemIdentity);
        assert_eq!(ordering.keys[1], SectionKey::UserRequest);
    }

    #[test]
    fn test_ordering_index_map() {
        let ordering = PromptOrdering::from_keys(vec![
            SectionKey::SystemIdentity,
            SectionKey::UserRequest,
            SectionKey::ProjectIdentity,
        ]);
        let map = ordering.index_map();
        assert_eq!(map[&SectionKey::SystemIdentity], 0);
        assert_eq!(map[&SectionKey::UserRequest], 1);
        assert_eq!(map[&SectionKey::ProjectIdentity], 2);
    }

    #[test]
    fn test_ordering_default_is_canonical() {
        let ordering = PromptOrdering::default();
        assert_eq!(ordering.keys, CANONICAL_ORDER);
    }
}

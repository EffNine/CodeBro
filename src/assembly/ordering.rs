use serde::{Deserialize, Serialize};

/// Canonical ordering of context sections in the assembled package.
/// Earlier sections appear first in the final output.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ContextSection {
    UserIntent,
    Workspace,
    RelevantFiles,
    EngineeringFacts,
    Diagnostics,
    GitChanges,
    Memory,
    ToolResults,
}

impl std::fmt::Display for ContextSection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContextSection::UserIntent => write!(f, "user_intent"),
            ContextSection::Workspace => write!(f, "workspace"),
            ContextSection::RelevantFiles => write!(f, "relevant_files"),
            ContextSection::EngineeringFacts => write!(f, "engineering_facts"),
            ContextSection::Diagnostics => write!(f, "diagnostics"),
            ContextSection::GitChanges => write!(f, "git_changes"),
            ContextSection::Memory => write!(f, "memory"),
            ContextSection::ToolResults => write!(f, "tool_results"),
        }
    }
}

impl ContextSection {
    /// Return the ordered list of sections in canonical priority order.
    pub fn canonical_order() -> Vec<ContextSection> {
        vec![
            ContextSection::UserIntent,
            ContextSection::Workspace,
            ContextSection::RelevantFiles,
            ContextSection::EngineeringFacts,
            ContextSection::Diagnostics,
            ContextSection::GitChanges,
            ContextSection::Memory,
            ContextSection::ToolResults,
        ]
    }

    /// Map a source to its canonical section.
    pub fn from_source(source: &crate::assembly::ContextSource) -> ContextSection {
        match source {
            crate::assembly::ContextSource::UserRequest => ContextSection::UserIntent,
            crate::assembly::ContextSource::Workspace => ContextSection::Workspace,
            crate::assembly::ContextSource::Indexer | crate::assembly::ContextSource::Scanner => {
                ContextSection::RelevantFiles
            }
            crate::assembly::ContextSource::EngineeringFacts => ContextSection::EngineeringFacts,
            crate::assembly::ContextSource::Git => ContextSection::GitChanges,
            crate::assembly::ContextSource::Memory => ContextSection::Memory,
            crate::assembly::ContextSource::ToolResults => ContextSection::ToolResults,
        }
    }
}

/// Re-order `fragments` so that they follow the canonical section order.
/// Within each section, fragments remain sorted by relevance score (desc).
pub fn order_fragments(fragments: &mut Vec<crate::assembly::ContextFragment>) {
    fragments.sort_by(|a, b| {
        let sa = ContextSection::from_source(&a.source);
        let sb = ContextSection::from_source(&b.source);
        sa.cmp(&sb).then_with(|| {
            b.relevance_score
                .partial_cmp(&a.relevance_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assembly::{ContextFragment, ContextPriority, ContextSource};

    fn fragment(source: ContextSource, score: f64, content: &str) -> ContextFragment {
        ContextFragment::new(source, ContextPriority::Medium, content.to_string(), score)
    }

    #[test]
    fn test_canonical_order() {
        let order = ContextSection::canonical_order();
        assert_eq!(order[0], ContextSection::UserIntent);
        assert_eq!(order[order.len() - 1], ContextSection::ToolResults);
    }

    #[test]
    fn test_from_source_mapping() {
        assert_eq!(
            ContextSection::from_source(&ContextSource::UserRequest),
            ContextSection::UserIntent
        );
        assert_eq!(
            ContextSection::from_source(&ContextSource::Git),
            ContextSection::GitChanges
        );
        assert_eq!(
            ContextSection::from_source(&ContextSource::Memory),
            ContextSection::Memory
        );
    }

    #[test]
    fn test_order_fragments() {
        let mut frags = vec![
            fragment(ContextSource::Memory, 0.9, "mem1"),
            fragment(ContextSource::UserRequest, 1.0, "req1"),
            fragment(ContextSource::Git, 0.8, "git1"),
            fragment(ContextSource::EngineeringFacts, 0.7, "fact1"),
        ];
        order_fragments(&mut frags);
        // Canonical order: UserIntent -> Workspace -> RelevantFiles -> EngineeringFacts -> Diagnostics -> GitChanges -> Memory -> ToolResults
        assert_eq!(frags[0].source, ContextSource::UserRequest);
        assert_eq!(frags[1].source, ContextSource::EngineeringFacts);
        assert_eq!(frags[2].source, ContextSource::Git);
        assert_eq!(frags[3].source, ContextSource::Memory);
    }

    #[test]
    fn test_order_fragments_within_section() {
        let mut frags = vec![
            fragment(ContextSource::Memory, 0.5, "mem_low"),
            fragment(ContextSource::Memory, 0.9, "mem_high"),
            fragment(ContextSource::Memory, 0.7, "mem_mid"),
        ];
        order_fragments(&mut frags);
        assert_eq!(frags[0].content, "mem_high");
        assert_eq!(frags[1].content, "mem_mid");
        assert_eq!(frags[2].content, "mem_low");
    }
}

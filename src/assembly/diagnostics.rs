use serde::{Deserialize, Serialize};

/// Diagnostic events emitted by the Context Assembly Engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AssemblyEvent {
    AssemblyStarted {
        request_hash: String,
    },
    FragmentCollected {
        source: String,
        count: usize,
    },
    FragmentDeduplicated {
        removed: usize,
    },
    FragmentBudgetTrimmed {
        removed: usize,
        remaining_tokens: usize,
    },
    AssemblyCompleted {
        elapsed_ms: u64,
        fragment_count: usize,
        estimated_tokens: usize,
    },
}

/// Observational diagnostics for the Context Assembly Engine.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssemblyDiagnostics {
    pub total_assemblies: u64,
    pub total_fragments_assembled: u64,
    pub total_tokens_delivered: u64,
    pub avg_elapsed_ms: f64,
    pub recent_events: Vec<AssemblyEvent>,
}

impl AssemblyDiagnostics {
    pub fn new() -> Self {
        AssemblyDiagnostics::default()
    }

    pub fn record_assembly(
        &mut self,
        fragment_count: usize,
        estimated_tokens: usize,
        elapsed_ms: u64,
    ) {
        self.total_assemblies += 1;
        self.total_fragments_assembled += fragment_count as u64;
        self.total_tokens_delivered += estimated_tokens as u64;
        let total = self.total_assemblies as f64;
        self.avg_elapsed_ms = (self.avg_elapsed_ms * (total - 1.0) + elapsed_ms as f64) / total;
    }

    pub fn summary(&self) -> String {
        format!(
            "Assembly Diagnostics:\n\
             Total assemblies: {}\n\
             Total fragments: {}\n\
             Total tokens delivered: {}\n\
             Average latency: {:.1} ms\n",
            self.total_assemblies,
            self.total_fragments_assembled,
            self.total_tokens_delivered,
            self.avg_elapsed_ms,
        )
    }
}

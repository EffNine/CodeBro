pub mod assembly;
pub mod budget;
pub mod config;
pub mod diagnostics;
pub mod intent;
pub mod ordering;
pub mod sources;
pub mod statistics;

pub use assembly::{ContextAssembler, ContextAssemblyRequest, ContextAssemblyResult};
pub use budget::{ContextBudget, TokenBudget};
pub use config::AssemblyConfig;
pub use diagnostics::AssemblyDiagnostics;
pub use intent::{IntentClassification, IntentType};
pub use ordering::ContextSection;
pub use sources::{
    ContextFragment, ContextPriority, ContextSource, EngineeringFactsSource, GitContextSource,
    IndexerContextSource, MemoryContextSource, ScannerContextSource, ToolResultsContextSource,
    UserRequestContextSource, WorkspaceContextSource,
};
pub use statistics::AssemblyStatistics;

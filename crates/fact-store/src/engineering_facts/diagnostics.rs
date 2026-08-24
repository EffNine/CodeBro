#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Diagnostic facts (P10.5.0).
//!
//! A diagnostic is an engineering finding — a warning or error a producer
//! discovered about the code under analysis. It is knowledge, not runtime
//! telemetry: no counters, timestamps or provider state live here.

use serde::{Deserialize, Serialize};

use crate::engineering_facts::ids::{DiagnosticId, FactId};
use crate::engineering_facts::location::SourceLocation;
use crate::engineering_facts::metadata::FactMetadata;
use crate::engineering_facts::types::Severity;

/// An engineering diagnostic fact. Immutable. Owned by the Engineering
/// Facts Model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticFact {
    pub id: DiagnosticId,
    pub severity: Severity,
    pub message: String,
    /// Optional stable diagnostic code (producer-defined).
    pub code: Option<String>,
    pub location: Option<SourceLocation>,
    /// Related facts (symbols, modules, rules) this diagnostic touches.
    pub related: Vec<FactId>,
    pub metadata: FactMetadata,
}

impl DiagnosticFact {
    pub fn new(id: DiagnosticId, severity: Severity, message: impl Into<String>) -> Self {
        DiagnosticFact {
            id,
            severity,
            message: message.into(),
            code: None,
            location: None,
            related: Vec::new(),
            metadata: FactMetadata::new(),
        }
    }
}

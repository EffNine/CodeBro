//! Intent semantics for P1: goals with rationale, distinct from preferences.
//!
//! An intent is a [`RecordKind::Intent`] context record whose `extra_json`
//! carries [`IntentMetadata`]:
//!
//! ```json
//! {"rationale": "why this goal matters", "priority": "high",
//!  "intent_status": "active"}
//! ```
//!
//! Intent status lives in `extra_json` — not as new [`RecordStatus`]
//! variants — so the existing storage lifecycle stays the single lifecycle
//! system: an actionable intent row is `RecordStatus::Active`; completing an
//! intent expires the row and cancelling rejects it (both terminal,
//! reversible via the audit trail); superseding marks it `Superseded` with
//! the replacement linked. Mapping:
//!
//! | Intent status | Record status |
//! |---------------|---------------|
//! | `active` / `paused` | `Active` (paused stays retrievable so OpenCode
//! sees what is on hold) |
//! | `completed` | `Expired` |
//! | `cancelled` | `Rejected` |
//! | `superseded` | `Superseded` |
//!
//! Preference ("prefer simple implementations"), decision ("use SQLite"),
//! and intent ("build CodeBro as the persistent layer for OpenCode") are
//! different concepts and must never collapse into one generic memory blob:
//! kinds keep them apart, `related_ids` links an intent to the decisions
//! taken under it.

use crate::types::{ContextRecord, RecordKind, RecordStatus};

/// Maximum characters of an intent rationale.
pub const MAX_RATIONALE_CHARS: usize = 1024;

/// Lifecycle of an intent (stored in `extra_json`, see module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentStatus {
    Active,
    Paused,
    Completed,
    Cancelled,
    Superseded,
}

impl IntentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            IntentStatus::Active => "active",
            IntentStatus::Paused => "paused",
            IntentStatus::Completed => "completed",
            IntentStatus::Cancelled => "cancelled",
            IntentStatus::Superseded => "superseded",
        }
    }

    /// Terminal statuses accept no further transitions: start a fresh
    /// intent instead of rewriting history.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            IntentStatus::Completed | IntentStatus::Cancelled | IntentStatus::Superseded
        )
    }

    /// Actionable statuses surface in the always-available context packet:
    /// what is active now plus what is explicitly on hold.
    pub fn is_actionable(&self) -> bool {
        matches!(self, IntentStatus::Active | IntentStatus::Paused)
    }
}

impl std::fmt::Display for IntentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for IntentStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "active" => Ok(IntentStatus::Active),
            "paused" => Ok(IntentStatus::Paused),
            "completed" => Ok(IntentStatus::Completed),
            "cancelled" | "canceled" => Ok(IntentStatus::Cancelled),
            "superseded" => Ok(IntentStatus::Superseded),
            other => Err(format!("unknown intent status: {other}")),
        }
    }
}

/// Relative importance of an intent (advisory ordering hint, not a rank).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentPriority {
    High,
    Medium,
    Low,
}

impl IntentPriority {
    pub fn as_str(&self) -> &'static str {
        match self {
            IntentPriority::High => "high",
            IntentPriority::Medium => "medium",
            IntentPriority::Low => "low",
        }
    }
}

impl std::fmt::Display for IntentPriority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for IntentPriority {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "high" => Ok(IntentPriority::High),
            "medium" | "med" => Ok(IntentPriority::Medium),
            "low" => Ok(IntentPriority::Low),
            other => Err(format!("unknown intent priority: {other}")),
        }
    }
}

/// Structured intent metadata (the `extra_json` payload for intent records).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntentMetadata {
    /// Why this goal matters (the "why" behind the "what").
    pub rationale: Option<String>,
    pub priority: Option<IntentPriority>,
    pub intent_status: IntentStatus,
}

impl Default for IntentMetadata {
    fn default() -> Self {
        IntentMetadata {
            rationale: None,
            priority: None,
            intent_status: IntentStatus::Active,
        }
    }
}

impl IntentMetadata {
    /// Decode from a record's `extra_json`. Absent metadata means a plain
    /// active intent (P0 rows predate the field); malformed metadata is an
    /// error, never silently defaulted.
    pub fn decode(extra_json: Option<&str>) -> Result<Self, String> {
        let raw = extra_json.unwrap_or("{}");
        let value: serde_json::Value =
            serde_json::from_str(raw).map_err(|e| format!("intent extra_json is not JSON: {e}"))?;
        let obj = value
            .as_object()
            .ok_or_else(|| "intent extra_json must be a JSON object".to_string())?;
        let rationale = obj
            .get("rationale")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        if let Some(r) = rationale.as_deref() {
            if r.chars().count() > MAX_RATIONALE_CHARS {
                return Err(format!(
                    "intent rationale exceeds {MAX_RATIONALE_CHARS} characters"
                ));
            }
        }
        let priority = obj
            .get("priority")
            .and_then(|v| v.as_str())
            .map(|s| s.parse::<IntentPriority>())
            .transpose()?;
        let intent_status = obj
            .get("intent_status")
            .and_then(|v| v.as_str())
            .map(|s| s.parse::<IntentStatus>())
            .transpose()?
            .unwrap_or(IntentStatus::Active);
        Ok(IntentMetadata {
            rationale,
            priority,
            intent_status,
        })
    }

    /// Encode into the `extra_json` string for storage.
    pub fn encode(&self) -> Result<String, String> {
        let mut map = serde_json::Map::new();
        if let Some(r) = self.rationale.as_deref() {
            if r.chars().count() > MAX_RATIONALE_CHARS {
                return Err(format!(
                    "intent rationale exceeds {MAX_RATIONALE_CHARS} characters"
                ));
            }
            map.insert(
                "rationale".to_string(),
                serde_json::Value::String(r.to_string()),
            );
        }
        if let Some(p) = self.priority {
            map.insert(
                "priority".to_string(),
                serde_json::Value::String(p.to_string()),
            );
        }
        map.insert(
            "intent_status".to_string(),
            serde_json::Value::String(self.intent_status.to_string()),
        );
        serde_json::to_string(&map).map_err(|e| e.to_string())
    }

    /// Attach to a record under construction (validates the kind).
    pub fn apply_to(&self, record: &mut ContextRecord) -> Result<(), String> {
        if record.kind != RecordKind::Intent {
            return Err("intent metadata applies only to intent records".to_string());
        }
        record.extra_json = Some(self.encode()?);
        Ok(())
    }

    /// Read from a stored intent record.
    pub fn read_from(record: &ContextRecord) -> Result<Self, String> {
        if record.kind != RecordKind::Intent {
            return Err("not an intent record".to_string());
        }
        Self::decode(record.extra_json.as_deref())
    }
}

/// Validate an intent status transition for a superseding replacement.
///
/// `None` (no predecessor) always passes — fresh intents start `active` or
/// `paused`. A terminal predecessor refuses: history is append-only, so a
/// finished intent is followed by a new record, never rewritten.
pub fn validate_transition(from: Option<IntentStatus>, _to: IntentStatus) -> Result<(), String> {
    match from {
        None => Ok(()),
        Some(prev) if prev.is_terminal() => Err(format!(
            "intent is {prev} (terminal): create a fresh intent instead of superseding it"
        )),
        Some(_) => Ok(()),
    }
}

/// Partition intent records into actionable / terminal / malformed.
///
/// Only `RecordStatus::Active` rows with an actionable `intent_status` are
/// actionable. Malformed rows (bad `extra_json`) are reported — never
/// silently defaulted — so corruption stays visible instead of masquerading
/// as an active goal.
pub fn partition_intents(
    records: &[crate::retrieval::RankedRecord],
) -> (
    Vec<&crate::retrieval::RankedRecord>,
    Vec<&crate::retrieval::RankedRecord>,
    Vec<&crate::retrieval::RankedRecord>,
) {
    let mut actionable = Vec::new();
    let mut terminal = Vec::new();
    let mut malformed = Vec::new();
    for ranked in records {
        if ranked.record.kind != RecordKind::Intent {
            continue;
        }
        if ranked.record.status != RecordStatus::Active {
            terminal.push(ranked);
            continue;
        }
        match IntentMetadata::read_from(&ranked.record) {
            Ok(meta) if meta.intent_status.is_actionable() => actionable.push(ranked),
            Ok(_) => terminal.push(ranked),
            Err(_) => malformed.push(ranked),
        }
    }
    (actionable, terminal, malformed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Authority, ContextRecord, RecordKind};

    fn intent_record() -> ContextRecord {
        ContextRecord::new(
            "ctx::intent-1",
            RecordKind::Intent,
            "intent.codebro-mission",
            "Build CodeBro as persistent engineering infrastructure for OpenCode",
            Authority::UserConfirmed,
        )
    }

    #[test]
    fn metadata_roundtrips_through_extra_json() {
        let meta = IntentMetadata {
            rationale: Some("context without another agent loop".to_string()),
            priority: Some(IntentPriority::High),
            intent_status: IntentStatus::Active,
        };
        let mut rec = intent_record();
        meta.apply_to(&mut rec).unwrap();
        assert_eq!(IntentMetadata::read_from(&rec).unwrap(), meta);
        assert!(crate::types::validate_record(&rec).is_ok());
    }

    #[test]
    fn absent_metadata_means_plain_active_intent() {
        let rec = intent_record();
        let meta = IntentMetadata::read_from(&rec).unwrap();
        assert_eq!(meta.intent_status, IntentStatus::Active);
        assert_eq!(meta.priority, None);
    }

    #[test]
    fn malformed_metadata_is_an_error_not_a_default() {
        let mut rec = intent_record();
        rec.extra_json = Some(r#"{"intent_status":"thriving"}"#.to_string());
        assert!(IntentMetadata::read_from(&rec).is_err());
        rec.extra_json = Some("[".to_string());
        assert!(IntentMetadata::read_from(&rec).is_err());
    }

    #[test]
    fn metadata_rejected_on_non_intent_kinds() {
        let mut rec = ContextRecord::new(
            "ctx::p",
            RecordKind::Preference,
            "fp.x",
            "content",
            Authority::UserConfirmed,
        );
        assert!(IntentMetadata::default().apply_to(&mut rec).is_err());
        assert!(IntentMetadata::read_from(&rec).is_err());
    }

    #[test]
    fn terminal_intents_refuse_supersession() {
        assert!(validate_transition(None, IntentStatus::Active).is_ok());
        assert!(validate_transition(Some(IntentStatus::Active), IntentStatus::Paused).is_ok());
        assert!(validate_transition(Some(IntentStatus::Paused), IntentStatus::Active).is_ok());
        assert!(validate_transition(Some(IntentStatus::Active), IntentStatus::Completed).is_ok());
        for done in [
            IntentStatus::Completed,
            IntentStatus::Cancelled,
            IntentStatus::Superseded,
        ] {
            assert!(
                validate_transition(Some(done), IntentStatus::Active).is_err(),
                "{done} must refuse"
            );
        }
    }

    #[test]
    fn status_and_priority_vocabularies() {
        assert_eq!(
            "paused".parse::<IntentStatus>().unwrap(),
            IntentStatus::Paused
        );
        assert!("thriving".parse::<IntentStatus>().is_err());
        assert_eq!(
            "high".parse::<IntentPriority>().unwrap(),
            IntentPriority::High
        );
        assert!("urgent".parse::<IntentPriority>().is_err());
        assert!(IntentStatus::Paused.is_actionable());
        assert!(!IntentStatus::Completed.is_actionable());
    }
}

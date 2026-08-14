//! Stdio JSON protocol for CodeBro <-> TUI communication

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuiRequest {
    pub id: u64,
    pub cmd: String,
    #[serde(default)]
    pub payload: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuiResponse {
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuiEvent {
    #[serde(rename = "event")]
    pub inner: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TuiMessage {
    Request(TuiRequest),
    Response(TuiResponse),
    Event(TuiEvent),
}

impl TuiMessage {
    pub fn parse(line: &str) -> Option<Self> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let v: serde_json::Value = serde_json::from_str(line).ok()?;
        if let Some(id) = v.get("id") {
            if v.get("cmd").is_some() {
                return Some(TuiMessage::Request(serde_json::from_value(v).ok()?));
            }
            if v.get("result").is_some() || v.get("error").is_some() {
                return Some(TuiMessage::Response(serde_json::from_value(v).ok()?));
            }
        }
        if v.get("event").is_some() {
            return Some(TuiMessage::Event(serde_json::from_value(v).ok()?));
        }
        None
    }

    pub fn to_writer(&self, writer: &mut impl std::io::Write) -> std::io::Result<()> {
        writeln!(writer, "{}", serde_json::to_string(self)?)
    }
}

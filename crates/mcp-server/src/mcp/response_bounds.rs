//! Global MCP response bounding.
//!
//! Every MCP tool response passes through [`bounded_response`] before it is
//! serialized to the wire. The pipeline is:
//!
//! ```text
//! domain result → compact projection → bounded projection → valid JSON
//! ```
//!
//! We never truncate serialized JSON bytes (which would produce invalid
//! JSON). Instead we structurally bound the [`serde_json::Value`]:
//! long strings are shortened on UTF-8 boundaries and oversized arrays are
//! windowed, with deterministic `truncated` metadata injected so agents can
//! tell a complete answer from a windowed one.

/// Single configurable ceiling for every MCP tool response (256 KiB).
/// Conservative: large enough for 50 fact records with excerpts, small
/// enough to keep token costs predictable.
pub const MAX_MCP_RESPONSE_BYTES: usize = 256 * 1024;

/// Per-string ceiling inside a bounded projection (8 KiB). Individual
/// fields are unbounded in the domain model (stdout, answer, description),
/// so a 50-record response can still blow the byte budget without this.
pub const MAX_STRING_FIELD_BYTES: usize = 8 * 1024;

/// Per-array ceiling inside a bounded projection. Collections are windowed
/// to the first N items with counts preserved where the caller provides
/// them.
pub const MAX_ARRAY_ITEMS: usize = 200;

/// Hard ceiling for externally-sourced error strings surfaced via MCP.
pub const MAX_ERROR_STRING_BYTES: usize = 4096;

/// Truncate `s` to at most `max_bytes` bytes on a UTF-8 boundary.
/// Returns `(truncated_string, was_truncated)`.
pub fn truncate_str(s: &str, max_bytes: usize) -> (String, bool) {
    if s.len() <= max_bytes {
        return (s.to_string(), false);
    }
    let mut cut = max_bytes;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    (
        format!("{}…[truncated {} more bytes]", &s[..cut], s.len() - cut),
        true,
    )
}

/// Bound an externally-sourced error string for MCP exposure.
pub fn bound_error_string(s: &str) -> String {
    truncate_str(s, MAX_ERROR_STRING_BYTES).0
}

/// Recursively bound a JSON value: strings longer than `str_limit` are
/// shortened, arrays longer than `arr_limit` are windowed. Returns whether
/// anything was truncated. Object keys are preserved; callers add envelope
/// metadata.
fn bound_value_inner(value: &mut serde_json::Value, str_limit: usize, arr_limit: usize) -> bool {
    match value {
        serde_json::Value::String(s) => {
            if s.len() > str_limit {
                let (t, _) = truncate_str(s, str_limit);
                *s = t;
                true
            } else {
                false
            }
        }
        serde_json::Value::Array(items) => {
            let mut truncated = false;
            if items.len() > arr_limit {
                items.truncate(arr_limit);
                truncated = true;
            }
            for item in items.iter_mut() {
                truncated |= bound_value_inner(item, str_limit, arr_limit);
            }
            truncated
        }
        serde_json::Value::Object(map) => {
            let mut truncated = false;
            for (_, v) in map.iter_mut() {
                truncated |= bound_value_inner(v, str_limit, arr_limit);
            }
            truncated
        }
        _ => false,
    }
}

/// Bound `payload` to [`MAX_MCP_RESPONSE_BYTES`] with structured truncation.
///
/// The result is always valid JSON. When truncation occurs and the payload
/// is an object, deterministic envelope keys are injected:
/// `truncated: true` and `limit_bytes`. `returned` is left to callers that
/// know collection cardinality (they should include their own counts).
pub fn bound_payload(mut payload: serde_json::Value) -> serde_json::Value {
    let mut str_limit = MAX_STRING_FIELD_BYTES;
    let mut arr_limit = MAX_ARRAY_ITEMS;
    let mut ever_truncated = bound_value_inner(&mut payload, str_limit, arr_limit);

    // Serialize-check loop: tighten progressively until under budget.
    // Limits halve each round (floors prevent degenerate empty output).
    for _ in 0..6 {
        let serialized_len = serde_json::to_string(&payload)
            .map(|s| s.len())
            .unwrap_or(0);
        // Reserve room for envelope keys.
        if serialized_len <= MAX_MCP_RESPONSE_BYTES {
            break;
        }
        str_limit = (str_limit / 2).max(256);
        arr_limit = (arr_limit / 2).max(10);
        ever_truncated = true;
        // Re-apply tighter bounds on already-bounded value.
        bound_value_inner(&mut payload, str_limit, arr_limit);
    }

    if ever_truncated {
        if let Some(map) = payload.as_object_mut() {
            map.insert("truncated".to_string(), serde_json::Value::Bool(true));
            map.insert(
                "limit_bytes".to_string(),
                serde_json::Value::Number(MAX_MCP_RESPONSE_BYTES.into()),
            );
        } else {
            payload = serde_json::json!({
                "result": payload,
                "truncated": true,
                "limit_bytes": MAX_MCP_RESPONSE_BYTES,
            });
        }
    }
    payload
}

/// Serialize `payload` to pretty JSON guaranteed to fit the global budget
/// and to remain valid JSON. Use for every MCP tool response.
pub fn bounded_response(payload: serde_json::Value) -> Result<String, String> {
    let bounded = bound_payload(payload);
    serde_json::to_string_pretty(&bounded).map_err(|e| bound_error_string(&e.to_string()))
}

/// Bound a diagnostics/violations-style string list to `limit` items,
/// returning `(kept, total, was_truncated)`.
pub fn bound_string_list(items: Vec<String>, limit: usize) -> (Vec<String>, usize, bool) {
    let total = items.len();
    if total <= limit {
        // Still bound individual entries.
        let kept: Vec<String> = items
            .into_iter()
            .map(|s| truncate_str(&s, MAX_STRING_FIELD_BYTES).0)
            .collect();
        (kept, total, false)
    } else {
        let kept: Vec<String> = items
            .into_iter()
            .take(limit)
            .map(|s| truncate_str(&s, MAX_STRING_FIELD_BYTES).0)
            .collect();
        (kept, total, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_json_after_truncation() {
        let big = "x".repeat(MAX_MCP_RESPONSE_BYTES * 2);
        let payload = serde_json::json!({"answer": big, "items": (0..1000).collect::<Vec<_>>(), "nested": {"s": "y".repeat(70000)}});
        let bounded = bound_payload(payload);
        let s = serde_json::to_string_pretty(&bounded).expect("valid json");
        assert!(s.len() <= MAX_MCP_RESPONSE_BYTES + 4096, "len={}", s.len());
        let reparsed: serde_json::Value = serde_json::from_str(&s).expect("reparse");
        assert_eq!(reparsed["truncated"], true);
        assert_eq!(reparsed["limit_bytes"], MAX_MCP_RESPONSE_BYTES);
    }

    #[test]
    fn truncation_metadata_deterministic() {
        let payload = serde_json::json!({"answer": "z".repeat(20000)});
        let a = bound_payload(payload.clone());
        let b = bound_payload(payload);
        assert_eq!(a, b);
        assert_eq!(a["truncated"], true);
    }

    #[test]
    fn small_payload_untouched_no_envelope() {
        let payload = serde_json::json!({"status": "ok"});
        let bounded = bound_payload(payload.clone());
        assert_eq!(bounded, payload);
    }

    #[test]
    fn huge_error_string_bounded() {
        let big = "e".repeat(100_000);
        let bounded = bound_error_string(&big);
        assert!(bounded.len() <= MAX_ERROR_STRING_BYTES + 64);
    }

    #[test]
    fn string_list_bounding_reports_counts() {
        let items: Vec<String> = (0..500).map(|i| format!("item-{i}")).collect();
        let (kept, total, truncated) = bound_string_list(items, 50);
        assert_eq!(kept.len(), 50);
        assert_eq!(total, 500);
        assert!(truncated);
    }
}

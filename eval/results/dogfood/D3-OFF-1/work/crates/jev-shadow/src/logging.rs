//! Structured JSONL shadow logging + secret scanning.
//!
//! Logs live OUTSIDE normal execution state: the default path is
//! `<workspace>/.codebro/jev-shadow.jsonl`, and `.codebro/` is excluded
//! from the working-tree hash, so shadow logging can never perturb
//! execution evidence. Records never contain the API key, authorization
//! headers, credentials, or raw sensitive state (state builders sanitize
//! before this point; the scanner below verifies after it).

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

use crate::shadow::ShadowRecord;
use std::path::Path;

/// Append one record as a single JSON line. Creates parent directories.
/// Best-effort by contract: callers ignore the result.
pub fn append_record(path: &Path, record: &ShadowRecord) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }
    use std::io::Write;
    let mut line = serde_json::to_string(record)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    line.push('\n');
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    f.write_all(line.as_bytes())?;
    Ok(())
}

/// Read back all records from a JSONL log (skips blank/corrupt lines with a
/// count so replay/analysis can report data quality honestly).
pub fn read_records(path: &Path) -> (Vec<ShadowRecord>, usize) {
    let mut out = Vec::new();
    let mut skipped = 0usize;
    let Ok(content) = std::fs::read_to_string(path) else {
        return (out, 0);
    };
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<ShadowRecord>(line) {
            Ok(r) => out.push(r),
            Err(_) => skipped += 1,
        }
    }
    (out, skipped)
}

/// Returns `true` if `secret` (non-empty) appears verbatim in `text`.
/// Used by the pilot's secret scan: the scan loads the real key from env
/// at scan time and asserts no log line contains it.
pub fn scan_text_for_secret(text: &str, secret: &str) -> bool {
    if secret.is_empty() {
        return false;
    }
    text.contains(secret)
}

/// Scan a log file for a verbatim secret. Returns the number of offending
/// lines (0 = clean). Never prints the secret.
pub fn scan_file_for_secret(path: &Path, secret: &str) -> usize {
    if secret.is_empty() {
        return 0;
    }
    let Ok(content) = std::fs::read_to_string(path) else {
        return 0;
    };
    content
        .lines()
        .filter(|l| scan_text_for_secret(l, secret))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_scan_detects_verbatim_key_and_ignores_empty() {
        assert!(scan_text_for_secret("Bearer abc123XYZ", "abc123XYZ"));
        assert!(!scan_text_for_secret("Bearer [REDACTED]", "abc123XYZ"));
        assert!(!scan_text_for_secret("anything", ""));
    }
}

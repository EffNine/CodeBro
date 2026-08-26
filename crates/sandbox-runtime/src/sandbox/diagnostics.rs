//! Structured parsing of build/test output into engineering diagnostics.
//!
//! [`VerificationResult`](super::VerificationResult) carries raw stdout/stderr
//! as evidence; this module adds a structured interpretation layer on top:
//!
//! - [`parse_diagnostics`] extracts compiler errors/warnings (rustc human
//!   format), Go compiler errors, cargo/pytest test failures, and panic
//!   locations from combined output.
//! - [`classify_failure`] reduces an execution plus its diagnostics to a
//!   coarse outcome (`success`, `compile_error`, `test_failure`, `timeout`,
//!   `denied`, `unknown_failure`) with deterministic precedence.
//!
//! Parsing is heuristic by design: it never invents facts — anything it
//! cannot confidently attribute stays unparsed raw evidence. It is bounded
//! ([`MAX_PARSE_LINES`]) so pathological output cannot stall the runtime.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

use serde::{Deserialize, Serialize};

/// Upper bound on output lines scanned per execution. Well beyond any real
/// build log under the 64 KiB output caps; guards against unbounded cost.
pub const MAX_PARSE_LINES: usize = 50_000;

/// One structured diagnostic extracted from build/test output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedDiagnostic {
    /// `error`, `warning`, or `failure` (test failure).
    pub severity: String,
    /// Compiler diagnostic code (e.g. `E0308`), when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Human-readable diagnostic message.
    pub message: String,
    /// Source file the diagnostic points at (workspace-relative when the
    /// toolchain emits relative paths, which is the default under cwd pinning).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    /// For failure-severity diagnostics: the failing test's name, when the
    /// runner output identifies it (cargo/go/pytest markers).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test: Option<String>,
}

impl ParsedDiagnostic {
    fn bare(severity: &str, message: impl Into<String>) -> Self {
        ParsedDiagnostic {
            severity: severity.to_string(),
            code: None,
            message: message.into(),
            file: None,
            line: None,
            column: None,
            test: None,
        }
    }

    fn failing_test(name: impl Into<String>) -> Self {
        let name = name.into();
        let mut d = Self::bare("failure", format!("test failed: {name}"));
        d.test = Some(name);
        d
    }
}

/// Extract structured diagnostics from combined build/test output.
pub fn parse_diagnostics(output: &str) -> Vec<ParsedDiagnostic> {
    let mut diags: Vec<ParsedDiagnostic> = Vec::new();
    let mut in_failures_block = false;
    for line in output.lines().take(MAX_PARSE_LINES) {
        if let Some(d) = parse_rustc_diagnostic(line) {
            diags.push(d);
            in_failures_block = false;
            continue;
        }
        if let Some((file, line_no, col)) = parse_span_line(line) {
            if let Some(last) = diags
                .iter_mut()
                .rev()
                .find(|d| d.file.is_none() && d.severity != "failure")
            {
                last.file = Some(file);
                last.line = Some(line_no);
                last.column = col;
            }
            continue;
        }
        if let Some(name) = parse_cargo_test_failure(line) {
            diags.push(ParsedDiagnostic::failing_test(name));
            continue;
        }
        match line.trim_end() {
            "failures:" => {
                in_failures_block = true;
                continue;
            }
            other if in_failures_block => {
                let name = other.trim();
                if name.is_empty()
                    || name.starts_with("test result")
                    || name.contains(':')
                    || name.starts_with('-')
                {
                    in_failures_block = false;
                } else {
                    // The block lists each failing test once; the earlier
                    // `test ... FAILED` line already recorded it.
                }
                continue;
            }
            _ => {}
        }
        in_failures_block = false;
        if let Some(d) = parse_panic_location(line) {
            attach_or_push(&mut diags, d);
            continue;
        }
        if let Some(d) = parse_go_diagnostic(line) {
            diags.push(d);
            continue;
        }
        if let Some(d) = parse_pytest_summary(line) {
            diags.push(d);
            continue;
        }
        if let Some(d) = parse_source_assertion_line(line) {
            diags.push(d);
        }
    }
    diags.dedup();
    diags
}

/// rustc human-format severity lines: `error[E0308]: msg`, `warning: msg`.
fn parse_rustc_diagnostic(line: &str) -> Option<ParsedDiagnostic> {
    let trimmed = line.trim_start();
    // Test-runner summaries ("error: test failed, to rerun pass …") are
    // outcome bookkeeping, not compiler diagnostics — the failure itself is
    // already captured as a `failure` diagnostic from the test result lines.
    if trimmed.starts_with("error: test failed") || trimmed.contains("to rerun pass") {
        return None;
    }
    let (severity, rest) = match trimmed.strip_prefix("error") {
        Some(r) => ("error", r),
        None => ("warning", trimmed.strip_prefix("warning")?),
    };
    let rest = rest.trim_start();
    let (code, message) = if let Some(after) = rest.strip_prefix('[') {
        let (code, tail) = after.split_once(']')?;
        (
            Some(code.to_string()),
            tail.trim_start_matches(':').trim().to_string(),
        )
    } else {
        let message = rest.strip_prefix(':')?.trim().to_string();
        (None, message)
    };
    if message.is_empty() {
        return None;
    }
    Some(ParsedDiagnostic {
        severity: severity.to_string(),
        code,
        message,
        file: None,
        line: None,
        test: None,
        column: None,
    })
}

/// rustc span lines: `--> src/lib.rs:12:34` (column optional).
fn parse_span_line(line: &str) -> Option<(String, u32, Option<u32>)> {
    let loc = line.trim().strip_prefix("-->")?.trim();
    let (path, line_no, col) = match loc.rsplit_once(':') {
        Some((before, c)) if c.parse::<u32>().is_ok() => {
            let col = c.parse::<u32>().ok();
            match before.rsplit_once(':') {
                Some((p, l)) if l.parse::<u32>().is_ok() => (p.to_string(), l.parse().ok(), col),
                _ => (before.to_string(), col, None),
            }
        }
        _ => return None,
    };
    let line_no = line_no?;
    if path.is_empty() {
        return None;
    }
    Some((path, line_no, col))
}

/// cargo test result lines: `test some::name ... FAILED`.
fn parse_cargo_test_failure(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("test ")?;
    let (name, verdict) = rest.split_once(" ... ")?;
    verdict
        .trim()
        .eq_ignore_ascii_case("FAILED")
        .then(|| name.to_string())
}

/// Rust panic location, new format:
/// `thread 'tests::it_works' panicked at src/lib.rs:5:5:` followed by the
/// message on the next line (handled by attaching later lines is out of
/// scope; the location itself is the evidence).
fn parse_panic_location(line: &str) -> Option<ParsedDiagnostic> {
    // Newer libtest embeds a thread id: `thread 'name' (12345) panicked at
    // src/lib.rs:15:9:`; older forms omit the parens. Locate the marker
    // directly instead of assuming the shape of what precedes it.
    let marker = line.find("panicked at ")?;
    let prefix = line[..marker].trim();
    let thread_name = prefix
        .strip_prefix("thread ")
        .and_then(|rest| {
            let quoted = rest.trim_start().strip_prefix('\'')?;
            quoted.split('\'').next().map(|n| n.to_string())
        })
        .unwrap_or_else(|| "<unknown>".to_string());
    let loc = line[marker + "panicked at ".len()..]
        .trim()
        .trim_end_matches(':');
    let (file, line_no, col) = split_file_line_col(loc)?;
    Some(ParsedDiagnostic {
        severity: "error".to_string(),
        code: Some("panic".to_string()),
        message: format!("panic in {thread_name}"),
        file: Some(file),
        line: Some(line_no),
        column: col,
        test: None,
    })
}

/// Go compile errors: `path/file.go:12:34: message`; also captures
/// `--- FAIL: TestName` test-failure markers.
fn parse_go_diagnostic(line: &str) -> Option<ParsedDiagnostic> {
    let trimmed = line.trim();
    if let Some(rest) = trimmed.strip_prefix("--- FAIL: ") {
        let name = rest.split_whitespace().next()?;
        return Some(ParsedDiagnostic::bare(
            "failure",
            format!("test failed: {name}"),
        ));
    }
    let (loc, message) = trimmed.split_once(": ")?;
    let (file, line_no, col) = split_file_line_col(loc)?;
    if !file.ends_with(".go") && !file.contains(".go:") {
        return None;
    }
    Some(ParsedDiagnostic {
        severity: "error".to_string(),
        code: None,
        message: message.to_string(),
        file: Some(file),
        line: Some(line_no),
        test: None,
        column: col,
    })
}

/// pytest short summary lines: `FAILED tests/test_x.py::test_y - AssertionError: …`.
fn parse_pytest_summary(line: &str) -> Option<ParsedDiagnostic> {
    let trimmed = line.trim();
    let (severity, rest) = match trimmed.strip_prefix("FAILED ") {
        Some(r) => ("failure", r),
        None => ("error", trimmed.strip_prefix("ERROR ")?),
    };
    let (node, message) = match rest.split_once(" - ") {
        Some((n, m)) => (n, m.to_string()),
        None => (rest, String::new()),
    };
    let mut d = ParsedDiagnostic::bare(
        severity,
        if message.is_empty() {
            format!("test failed: {node}")
        } else {
            message
        },
    );
    if let Some((file, tail)) = node.split_once("::") {
        d.file = Some(file.to_string());
        // Node-id tail is the test name (possibly ::parametrized).
        d.test = Some(tail.to_string());
    } else {
        d.test = Some(node.to_string());
    }
    Some(d)
}

/// Runtime assertion frames from long tracebacks:
/// `helpers.py:3: AssertionError`. Restricted to known source extensions
/// and requiring a message, so arbitrary path-ish text is not invented
/// into diagnostics.
fn parse_source_assertion_line(line: &str) -> Option<ParsedDiagnostic> {
    const SOURCE_EXTS: &[&str] = &[".py", ".rs", ".ts", ".tsx", ".js", ".jsx", ".go"];
    let trimmed = line.trim();
    if trimmed.starts_with('-') || trimmed.starts_with('>') {
        // Pytest caret/arrow context lines carry no message of their own.
        return None;
    }
    let (loc, message) = trimmed.split_once(": ")?;
    if message.is_empty() {
        return None;
    }
    let (file, line_no, col) = split_file_line_col(loc)?;
    if !SOURCE_EXTS.iter().any(|ext| file.ends_with(ext)) {
        return None;
    }
    Some(ParsedDiagnostic {
        severity: "error".to_string(),
        code: None,
        message: message.to_string(),
        file: Some(file),
        line: Some(line_no),
        test: None,
        column: col,
    })
}

/// Split `path:line[:col]`, tolerating paths without colons and returning
/// None unless at least a numeric line is present.
fn split_file_line_col(loc: &str) -> Option<(String, u32, Option<u32>)> {
    let (before, last) = loc.rsplit_once(':')?;
    if let Ok(n) = last.parse::<u32>() {
        // Two numeric suffixes → line:col; one → line only.
        if let Some((f, l)) = before.rsplit_once(':') {
            if let Ok(line) = l.parse::<u32>() {
                return Some((f.to_string(), line, Some(n)));
            }
        }
        return Some((before.to_string(), n, None));
    }
    None
}

/// Backfill a panic location onto a preceding bare failure entry when
/// plausible, AND keep the panic itself as a distinct coded diagnostic.
fn attach_or_push(diags: &mut Vec<ParsedDiagnostic>, d: ParsedDiagnostic) {
    if let Some(last) = diags
        .iter_mut()
        .rev()
        .find(|x| x.severity == "failure" && x.file.is_none())
    {
        last.file = d.file.clone();
        last.line = d.line;
        last.column = d.column;
    }
    diags.push(d);
}

/// Reduce an execution outcome + parsed diagnostics to a coarse
/// classification. Precedence: denied → timeout → success → compile_error →
/// test_failure → unknown_failure. Compile detection requires positive
/// evidence (a diagnostic code, a source span, or cargo's own "could not
/// compile" summary) so unrelated scripts printing "error: …" are not
/// misclassified.
pub fn classify_failure(
    success: bool,
    denied: bool,
    timeout: bool,
    diags: &[ParsedDiagnostic],
) -> &'static str {
    if denied {
        return "denied";
    }
    if timeout {
        return "timeout";
    }
    if success {
        return "success";
    }
    let compile_evidence = diags.iter().any(|d| match d.code.as_deref() {
        // Synthetic panic code means a test failure, not compilation.
        Some("panic") => false,
        Some(c) => c.starts_with('E'),
        None => {
            d.message.contains("could not compile")
                // Codeless diagnostics need a COLUMN to be compiler-shaped
                // (rustc/go spans always carry one); runtime assertion
                // frames (`helpers.py:3: AssertionError`) do not.
                || (d.severity == "error" && d.file.is_some() && d.column.is_some())
        }
    });
    if compile_evidence {
        return "compile_error";
    }
    if diags.iter().any(|d| d.severity == "failure") {
        return "test_failure";
    }
    "unknown_failure"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rustc_errors_with_codes_and_spans() {
        let output = "\
error[E0308]: mismatched types
 --> src/lib.rs:12:34
  |
12 |     let x: i32 = \"s\";
   |                  ^^^ expected `i32`, found `&str`

warning: unused variable: `y`
 --> src/main.rs:3:9
";
        let diags = parse_diagnostics(output);
        assert_eq!(diags.len(), 2);
        assert_eq!(diags[0].severity, "error");
        assert_eq!(diags[0].code.as_deref(), Some("E0308"));
        assert_eq!(diags[0].file.as_deref(), Some("src/lib.rs"));
        assert_eq!(diags[0].line, Some(12));
        assert_eq!(diags[0].column, Some(34));
        assert_eq!(diags[1].severity, "warning");
        assert_eq!(diags[1].file.as_deref(), Some("src/main.rs"));
    }

    #[test]
    fn captures_cargo_test_failures_and_panics() {
        let output = "\
test ops::adds_numbers ... ok
test ops::breaks ... FAILED
failures:

failures:
    ops::breaks

thread 'ops::breaks' panicked at src/ops.rs:7:5:
assertion `left == right` failed
";
        let diags = parse_diagnostics(output);
        let failure = diags
            .iter()
            .find(|d| d.severity == "failure")
            .expect("test failure captured");
        assert_eq!(failure.message, "test failed: ops::breaks");
        assert_eq!(failure.file.as_deref(), Some("src/ops.rs"));
        assert_eq!(failure.line, Some(7));
        let panic_diag = diags
            .iter()
            .find(|d| d.code.as_deref() == Some("panic"))
            .expect("panic captured");
        assert_eq!(panic_diag.file.as_deref(), Some("src/ops.rs"));
        assert_eq!(panic_diag.message, "panic in ops::breaks");
        // Runner summaries must not leak in as compiler errors.
        assert!(diags.iter().all(|d| !d.message.contains("to rerun pass")));
    }

    #[test]
    fn parses_go_compile_errors_and_test_markers() {
        let output = "\
./math/math.go:10:2: undefined: helper
--- FAIL: TestDivide (0.00s)
    math.go:14: bad divide
";
        let diags = parse_diagnostics(output);
        assert_eq!(diags.len(), 3);
        assert_eq!(diags[0].severity, "error");
        assert_eq!(diags[0].file.as_deref(), Some("./math/math.go"));
        assert_eq!(diags[0].line, Some(10));
        assert_eq!(diags[1].severity, "failure");
        assert_eq!(diags[1].message, "test failed: TestDivide");
        // Go t.Fatal/t.Error log lines carry file:line evidence too — but
        // are RUNTIME failures, not compile evidence.
        assert_eq!(diags[2].file.as_deref(), Some("math.go"));
        assert_eq!(diags[2].line, Some(14));
        assert_eq!(diags[2].message, "bad divide");
        // Combined output: genuine compiler line wins the classification.
        assert_eq!(
            classify_failure(false, false, false, &diags),
            "compile_error"
        );
        // Pure runtime failure (log line only, no column): test_failure.
        let runtime_only =
            parse_diagnostics("--- FAIL: TestDivide (0.00s)\n    math.go:14: bad divide\n");
        assert_eq!(
            classify_failure(false, false, false, &runtime_only),
            "test_failure"
        );
    }

    #[test]
    fn parses_pytest_summary_lines() {
        let output = "FAILED tests/test_math.py::test_divide - ZeroDivisionError: division by zero\nERROR tests/test_io.py::test_read\n";
        let diags = parse_diagnostics(output);
        assert_eq!(diags.len(), 2);
        assert_eq!(diags[0].severity, "failure");
        assert_eq!(diags[0].message, "ZeroDivisionError: division by zero");
        assert_eq!(diags[0].file.as_deref(), Some("tests/test_math.py"));
        assert_eq!(diags[1].severity, "error");

        // Long-traceback source frame lines attribute the failure to the
        // mutated module, not just the test file.
        let noise = parse_diagnostics("notes.md:5: AssertionError\n");
        assert!(noise.is_empty(), "non-source extensions must be ignored");
        let frame = parse_diagnostics("helpers.py:3: AssertionError\n");
        assert_eq!(frame.len(), 1);
        assert_eq!(frame[0].file.as_deref(), Some("helpers.py"));
        assert_eq!(frame[0].line, Some(3));
        assert_eq!(
            classify_failure(false, false, false, &frame),
            "unknown_failure",
            "bare assertion frames alone are not compile evidence"
        );
    }

    #[test]
    fn classification_precedence_is_deterministic() {
        let err = ParsedDiagnostic::bare("error", "mismatched types");
        let mut coded = err.clone();
        coded.code = Some("E0308".to_string());
        let fail = ParsedDiagnostic::bare("failure", "test failed: x");

        assert_eq!(
            classify_failure(false, true, false, &[coded.clone()]),
            "denied"
        );
        assert_eq!(classify_failure(false, false, true, &[coded]), "timeout");
        assert_eq!(classify_failure(true, false, false, &[]), "success");
        assert_eq!(
            classify_failure(
                false,
                false,
                false,
                &[ParsedDiagnostic {
                    code: Some("E999".to_string()),
                    ..err
                }]
            ),
            "compile_error"
        );
        assert_eq!(
            classify_failure(false, false, false, &[fail]),
            "test_failure"
        );
        assert_eq!(
            classify_failure(false, false, false, &[]),
            "unknown_failure"
        );
    }

    #[test]
    fn plain_error_lines_without_evidence_are_not_compile_errors() {
        let output = "error: something went wrong\nexit status 1\n";
        let diags = parse_diagnostics(output);
        assert!(diags.len() == 1);
        assert_eq!(
            classify_failure(false, false, false, &diags),
            "unknown_failure"
        );
    }

    #[test]
    fn could_not_compile_is_classified_as_compile_error() {
        let output = "error: could not compile `probe` (bin \"probe\")\n";
        let diags = parse_diagnostics(output);
        assert_eq!(
            classify_failure(false, false, false, &diags),
            "compile_error"
        );
    }
}

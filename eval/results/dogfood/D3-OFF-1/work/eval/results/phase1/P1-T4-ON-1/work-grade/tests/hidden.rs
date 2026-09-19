//! T4 held-out hidden tests. The agent NEVER sees this file during the task.
//! `grade.sh` copies it to `<fixture>/tests/hidden.rs` at grading time.
//!
//! These tests execute the compiled server binary with piped stdin and assert
//! the §5 deployment invariant: stdout carries ONLY protocol JSON lines.
use std::io::Write;
use std::process::{Command, Stdio};

fn run_server(input: &str) -> (String, String) {
    let bin = env!("CARGO_BIN_EXE_qserver");
    let mut child = Command::new(bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn qserver");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait");
    (
        String::from_utf8(out.stdout).expect("stdout utf8"),
        String::from_utf8(out.stderr).expect("stderr utf8"),
    )
}

#[test]
fn hidden_stdout_is_pure_json_lines() {
    let input = "{\"cmd\":\"ping\"}\n{\"cmd\":\"store\",\"key\":\"a\",\"val\":\"1\"}\n{\"cmd\":\"get\",\"key\":\"a\"}\n{\"cmd\":\"get\",\"key\":\"zz\"}\ngarbage\n";
    let (stdout, _stderr) = run_server(input);
    let lines: Vec<&str> = stdout.lines().collect();
    // One response line per input line, nothing else.
    assert_eq!(lines.len(), 5, "stdout must carry exactly one line per request, got: {:?}", lines);
    for line in &lines {
        assert!(
            line.starts_with('{') && line.ends_with('}'),
            "every stdout line must be a JSON protocol line, got: {:?}",
            line
        );
    }
    assert_eq!(lines[0], "{\"ok\":true,\"pong\":true}");
    assert_eq!(lines[2], "{\"ok\":true,\"val\":\"1\"}");
    assert_eq!(lines[3], "{\"ok\":false,\"err\":\"missing\"}");
    assert_eq!(lines[4], "{\"ok\":false,\"err\":\"bad-request\"}");
}

#[test]
fn hidden_diagnostics_exist_on_stderr() {
    // Five requests must produce diagnostics SOMEWHERE other than stdout.
    let input = "{\"cmd\":\"ping\"}\n{\"cmd\":\"ping\"}\n{\"cmd\":\"ping\"}\n{\"cmd\":\"ping\"}\n{\"cmd\":\"ping\"}\n";
    let (stdout, stderr) = run_server(input);
    assert_eq!(stdout.lines().count(), 5);
    assert!(
        stderr.lines().count() >= 5,
        "expected at least one diagnostic per request on stderr, got: {:?}",
        stderr
    );
}

#[test]
fn hidden_no_log_marker_on_stdout() {
    // The stub's `LOG ...` diagnostic marker must never reach stdout.
    let input = "{\"cmd\":\"store\",\"key\":\"k\",\"val\":\"v\"}\n";
    let (stdout, _) = run_server(input);
    assert!(
        !stdout.contains("LOG"),
        "diagnostic marker leaked to stdout: {:?}",
        stdout
    );
}

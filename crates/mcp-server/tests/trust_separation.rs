//! Trust-model invariant: engineering memory writes must never touch the
//! verified fact store. The two stores are structurally separate; this test
//! pins that separation at the filesystem boundary.

use std::path::Path;

fn repo_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repo root")
        .to_path_buf()
}

/// The memory runtime source must not reference fact-store persistence.
#[test]
fn memory_runtime_has_no_path_into_facts() {
    let mem_dir = repo_root().join("crates/memory-runtime/src");
    for entry in walkdir::WalkDir::new(mem_dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
    {
        let content = std::fs::read_to_string(entry.path()).unwrap_or_default();
        for line in content.lines() {
            let t = line.trim_start();
            if t.starts_with("//") {
                continue;
            }
            assert!(
                !t.contains("facts.json"),
                "memory runtime must never read or write facts.json: {}",
                entry.path().display()
            );
            assert!(
                !t.contains("FactStore") && !t.contains("FactsBuilder"),
                "memory runtime must not construct fact-store types: {}",
                entry.path().display()
            );
        }
    }
}

/// Recording memory leaves the fact store bytes untouched.
#[test]
fn recording_memory_preserves_fact_store_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let codebro = dir.path().join(".codebro");
    std::fs::create_dir_all(&codebro).unwrap();
    let facts_path = codebro.join("facts.json");
    let before = br#"{"workspaces":[],"total":0,"marker":"untouched"}"#;
    std::fs::write(&facts_path, before).unwrap();

    // Record memory through the canonical runtime seam.
    let mut identity =
        codebro_identity_runtime::project_identity::ProjectIdentityRuntime::new(dir.path());
    let _ = identity.load();
    let mut memory = codebro_memory_runtime::engineering_memory::EngineeringMemoryRuntime::new(
        dir.path(),
        identity,
    );
    let entry = codebro_memory_runtime::engineering_memory::types::EngineeringMemoryEntry::new(
        "mem::sep-test",
        "separation:test",
        "memory stays out of the fact store",
    );
    memory.record(entry).expect("record succeeds");
    memory.persist().expect("persist succeeds");

    let after = std::fs::read(&facts_path).unwrap();
    assert_eq!(
        &after[..],
        &before[..],
        "facts.json was modified by a memory write"
    );
}

/// The context runtime source must not reference the JSON stores either:
/// user-context rows live in SQLite and may only *reference* canonical
/// entities via related_ids — never read or write the files.
#[test]
fn context_runtime_has_no_path_into_json_stores() {
    let ctx_dir = repo_root().join("crates/context-runtime/src");
    for entry in walkdir::WalkDir::new(ctx_dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
    {
        let content = std::fs::read_to_string(entry.path()).unwrap_or_default();
        for line in content.lines() {
            let t = line.trim_start();
            if t.starts_with("//") {
                continue;
            }
            for forbidden in [
                "facts.json",
                "engineering_memory.json",
                "project_identity.json",
                "FactStore",
                "FactsBuilder",
                "EngineeringMemoryRuntime",
                "ProjectIdentityRuntime",
            ] {
                assert!(
                    !t.contains(forbidden),
                    "context runtime must not touch {forbidden}: {}",
                    entry.path().display()
                );
            }
        }
    }
}

/// Writing context records leaves every project state file untouched.
#[test]
fn context_writes_preserve_project_state_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let codebro = dir.path().join(".codebro");
    std::fs::create_dir_all(&codebro).unwrap();
    let markers: Vec<(std::path::PathBuf, &[u8])> = vec![
        (
            codebro.join("facts.json"),
            br#"{"marker":"facts-untouched"}"#,
        ),
        (
            codebro.join("engineering_memory.json"),
            br#"{"marker":"memory-untouched"}"#,
        ),
        (
            codebro.join("project_identity.json"),
            br#"{"marker":"identity-untouched"}"#,
        ),
    ];
    for (path, bytes) in &markers {
        std::fs::write(path, bytes).unwrap();
    }

    // Write through the canonical store seam: record + supersede + event.
    let state = tempfile::tempdir().unwrap();
    let store = codebro_context_runtime::ContextStore::at_state_dir(state.path().to_path_buf());
    let ws = dir.path().display().to_string();
    let mut rec = codebro_context_runtime::ContextRecord::new(
        "ctx::sep",
        codebro_context_runtime::RecordKind::Preference,
        "fp.separation",
        "context stays out of the project stores",
        codebro_context_runtime::Authority::UserConfirmed,
    );
    rec.scope = codebro_context_runtime::RecordScope::Project;
    rec.workspace_root = Some(ws);
    store.put_record(&rec, 1).expect("put succeeds");
    let mut next = rec.clone();
    next.id = "ctx::sep-2".to_string();
    next.content = "revised".to_string();
    next.supersedes = Some("ctx::sep".to_string());
    store
        .supersede_record("ctx::sep", &next, 2)
        .expect("supersede succeeds");
    store
        .append_event(
            &codebro_context_runtime::EventRecord {
                id: None,
                session_id: None,
                workspace_root: dir.path().display().to_string(),
                task_id: None,
                kind: "separation_probe".to_string(),
                tool: None,
                path: None,
                outcome: None,
                summary: None,
                payload: None,
                dedup_key: None,
                source: None,
                digest: None,
                created_at: 0,
            },
            3,
        )
        .expect("event succeeds");

    for (path, before) in &markers {
        let after = std::fs::read(path).unwrap();
        assert_eq!(
            &after[..],
            &before[..],
            "{} was modified by a context write",
            path.display()
        );
    }
}

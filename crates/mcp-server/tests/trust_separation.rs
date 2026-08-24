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

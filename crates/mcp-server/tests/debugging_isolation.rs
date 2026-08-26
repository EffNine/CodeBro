//! Phase-8 isolation guard: the debugging engine consumes fact-store,
//! impact, and sandbox evidence only. Engineering memory must never appear
//! as a root-cause input.

#[test]
fn debugging_never_references_engineering_memory() {
    let mut offenders = Vec::new();
    for entry in std::fs::read_dir("src/debugging").expect("debugging dir") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("read");
        for (idx, line) in src.lines().enumerate() {
            let lower = line.to_lowercase();
            if lower.contains("engineering_memory")
                || lower.contains("engineeringmemory")
                || lower.contains("record_memory")
            {
                // Comment lines explaining the invariant are fine.
                if line.trim_start().starts_with("//") {
                    continue;
                }
                offenders.push(format!("{}:{}: {}", path.display(), idx + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "debugging module must not consume engineering memory:\n{}",
        offenders.join("\n")
    );
}

//! Workflow Dependencies — dependency graph construction and analysis.
//!
/// Builds dependency graphs from workflow steps.
/// Detects cycles, missing dependencies, and ordering issues.
/// No execution, no state mutation.
use super::types::*;

/// Build dependency relationships from workflow steps.
///
/// Returns a list of WorkflowDependency objects.
pub fn build_dependencies(steps: &[WorkflowStep]) -> Vec<WorkflowDependency> {
    let mut deps = Vec::new();

    // Build a map of step IDs for quick lookup
    let step_ids: std::collections::HashSet<&String> = steps.iter().map(|s| &s.step_id).collect();

    for step in steps {
        // Check each declared dependency
        for dep_id in &step.dependencies {
            if step_ids.contains(dep_id) {
                deps.push(WorkflowDependency {
                    from_step: dep_id.clone(),
                    to_step: step.step_id.clone(),
                    dependency_type: DependencyType::MustCompleteBefore,
                });
            }
        }
    }

    deps
}

/// Check if there are any dependency cycles in the graph.
///
/// Returns true if a cycle is detected, false otherwise.
pub fn has_cycles(steps: &[WorkflowStep], dependencies: &[WorkflowDependency]) -> bool {
    let mut adj: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for dep in dependencies {
        adj.entry(dep.to_step.clone())
            .or_insert_with(Vec::new)
            .push(dep.from_step.clone());
    }

    let mut visited = std::collections::HashSet::new();
    let mut rec_stack = std::collections::HashSet::new();

    for step in steps {
        if !visited.contains(&step.step_id) {
            if dfs_cycle(&step.step_id, &adj, &mut visited, &mut rec_stack) {
                return true;
            }
        }
    }

    false
}

fn dfs_cycle(
    node: &str,
    adj: &std::collections::HashMap<String, Vec<String>>,
    visited: &mut std::collections::HashSet<String>,
    rec_stack: &mut std::collections::HashSet<String>,
) -> bool {
    visited.insert(node.to_string());
    rec_stack.insert(node.to_string());

    if let Some(neighbors) = adj.get(node) {
        for neighbor in neighbors {
            if !visited.contains(neighbor) {
                if dfs_cycle(neighbor, adj, visited, rec_stack) {
                    return true;
                }
            } else if rec_stack.contains(neighbor) {
                return true;
            }
        }
    }

    rec_stack.remove(node);
    false
}

/// Find all steps that a given step depends on (transitively).
pub fn find_transitive_dependencies(
    step_id: &str,
    dependencies: &[WorkflowDependency],
) -> Vec<String> {
    let mut result = Vec::new();
    let mut visited = std::collections::HashSet::new();
    collect_dependencies(step_id, dependencies, &mut result, &mut visited);
    result
}

fn collect_dependencies(
    step_id: &str,
    dependencies: &[WorkflowDependency],
    result: &mut Vec<String>,
    visited: &mut std::collections::HashSet<String>,
) {
    if !visited.insert(step_id.to_string()) {
        return;
    }

    for dep in dependencies {
        if dep.to_step == step_id {
            result.push(dep.from_step.clone());
            collect_dependencies(&dep.from_step, dependencies, result, visited);
        }
    }
}

/// Find all steps that depend on a given step (transitively).
pub fn find_transitive_dependents(
    step_id: &str,
    dependencies: &[WorkflowDependency],
) -> Vec<String> {
    let mut result = Vec::new();
    let mut visited = std::collections::HashSet::new();
    collect_dependents(step_id, dependencies, &mut result, &mut visited);
    result
}

fn collect_dependents(
    step_id: &str,
    dependencies: &[WorkflowDependency],
    result: &mut Vec<String>,
    visited: &mut std::collections::HashSet<String>,
) {
    if !visited.insert(step_id.to_string()) {
        return;
    }

    for dep in dependencies {
        if dep.from_step == step_id {
            result.push(dep.to_step.clone());
            collect_dependents(&dep.to_step, dependencies, result, visited);
        }
    }
}

/// Find steps with no dependencies (entry points).
pub fn find_entry_points(
    steps: &[WorkflowStep],
    dependencies: &[WorkflowDependency],
) -> Vec<String> {
    let dependent_ids: std::collections::HashSet<&str> =
        dependencies.iter().map(|d| d.to_step.as_str()).collect();
    steps
        .iter()
        .filter(|s| !dependent_ids.contains(s.step_id.as_str()))
        .map(|s| s.step_id.clone())
        .collect()
}

/// Find steps with no dependents (exit points).
pub fn find_exit_points(
    steps: &[WorkflowStep],
    dependencies: &[WorkflowDependency],
) -> Vec<String> {
    let from_ids: std::collections::HashSet<&str> =
        dependencies.iter().map(|d| d.from_step.as_str()).collect();
    steps
        .iter()
        .filter(|s| !from_ids.contains(s.step_id.as_str()))
        .map(|s| s.step_id.clone())
        .collect()
}

/// Calculate the depth of the dependency graph.
///
/// Returns the maximum chain length.
pub fn calculate_depth(steps: &[WorkflowStep], dependencies: &[WorkflowDependency]) -> usize {
    let entry_points = find_entry_points(steps, dependencies);
    if entry_points.is_empty() {
        return if steps.is_empty() { 0 } else { 1 };
    }

    let mut max_depth = 0;
    for entry in &entry_points {
        let depth = dfs_depth(entry, dependencies, &mut std::collections::HashMap::new());
        max_depth = max_depth.max(depth);
    }
    max_depth
}

fn dfs_depth(
    node: &str,
    dependencies: &[WorkflowDependency],
    memo: &mut std::collections::HashMap<String, usize>,
) -> usize {
    if let Some(&depth) = memo.get(node) {
        return depth;
    }

    let dependents: Vec<String> = dependencies
        .iter()
        .filter(|d| d.from_step == node)
        .map(|d| d.to_step.clone())
        .collect();

    let max_child_depth = dependents
        .iter()
        .map(|dep| dfs_depth(dep, dependencies, memo))
        .max()
        .unwrap_or(0);

    let depth = max_child_depth + 1;
    memo.insert(node.to_string(), depth);
    depth
}

/// Check if adding a new dependency would create a cycle.
pub fn would_create_cycle(
    from_step: &str,
    to_step: &str,
    existing_deps: &[WorkflowDependency],
) -> bool {
    // Check if to_step is already a transitive dependency of from_step
    let transitive_deps = find_transitive_dependencies(from_step, existing_deps);
    transitive_deps.contains(&to_step.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_step(id: &str, deps: Vec<&str>) -> WorkflowStep {
        WorkflowStep {
            step_id: id.to_string(),
            name: id.to_string(),
            command: format!("cmd_{}", id),
            stage: WorkflowStage::Execution,
            priority: 0,
            dependencies: deps.into_iter().map(|s| s.to_string()).collect(),
            requires_approval: false,
            estimated_cost: 0.0,
            reversible: true,
            description: "Test step".to_string(),
        }
    }

    #[test]
    fn test_build_dependencies_empty() {
        let steps: Vec<WorkflowStep> = vec![];
        let deps = build_dependencies(&steps);
        assert!(deps.is_empty());
    }

    #[test]
    fn test_build_dependencies_single() {
        let steps = vec![make_step("a", vec![])];
        let deps = build_dependencies(&steps);
        assert!(deps.is_empty());
    }

    #[test]
    fn test_build_dependencies_with_deps() {
        let steps = vec![make_step("a", vec![]), make_step("b", vec!["a"])];
        let deps = build_dependencies(&steps);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].from_step, "a");
        assert_eq!(deps[0].to_step, "b");
    }

    #[test]
    fn test_has_cycles_none() {
        let steps = vec![
            make_step("a", vec![]),
            make_step("b", vec!["a"]),
            make_step("c", vec!["b"]),
        ];
        let deps = build_dependencies(&steps);
        assert!(!has_cycles(&steps, &deps));
    }

    #[test]
    fn test_has_cycles_direct() {
        let steps = vec![make_step("a", vec!["b"]), make_step("b", vec!["a"])];
        let deps = build_dependencies(&steps);
        assert!(has_cycles(&steps, &deps));
    }

    #[test]
    fn test_has_cycles_indirect() {
        let steps = vec![
            make_step("a", vec!["c"]),
            make_step("b", vec!["a"]),
            make_step("c", vec!["b"]),
        ];
        let deps = build_dependencies(&steps);
        assert!(has_cycles(&steps, &deps));
    }

    #[test]
    fn test_find_entry_points() {
        let steps = vec![
            make_step("a", vec![]),
            make_step("b", vec!["a"]),
            make_step("c", vec!["a"]),
        ];
        let deps = build_dependencies(&steps);
        let entries = find_entry_points(&steps, &deps);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0], "a");
    }

    #[test]
    fn test_find_exit_points() {
        let steps = vec![
            make_step("a", vec![]),
            make_step("b", vec!["a"]),
            make_step("c", vec!["b"]),
        ];
        let deps = build_dependencies(&steps);
        let exits = find_exit_points(&steps, &deps);
        assert_eq!(exits.len(), 1);
        assert_eq!(exits[0], "c");
    }

    #[test]
    fn test_calculate_depth() {
        let steps = vec![
            make_step("a", vec![]),
            make_step("b", vec!["a"]),
            make_step("c", vec!["b"]),
        ];
        let deps = build_dependencies(&steps);
        let depth = calculate_depth(&steps, &deps);
        assert_eq!(depth, 3);
    }

    #[test]
    fn test_would_create_cycle_false() {
        assert!(!would_create_cycle("a", "b", &[]));
    }

    #[test]
    fn test_would_create_cycle_true() {
        let deps = vec![
            WorkflowDependency {
                from_step: "a".to_string(),
                to_step: "b".to_string(),
                dependency_type: DependencyType::MustCompleteBefore,
            },
            WorkflowDependency {
                from_step: "b".to_string(),
                to_step: "c".to_string(),
                dependency_type: DependencyType::MustCompleteBefore,
            },
        ];
        assert!(would_create_cycle("c", "a", &deps));
    }
}

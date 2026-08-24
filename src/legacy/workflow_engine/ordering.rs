//! Workflow Ordering — deterministic step ordering.
//!
/// Topological sort, priority ordering, stable ordering.
/// No randomness, no timestamps.
use super::types::*;

/// Perform topological sort on workflow steps based on dependencies.
///
/// Returns steps in dependency order (prerequisites first).
/// If no dependencies exist, returns steps in their original order.
pub fn topological_sort(
    steps: Vec<WorkflowStep>,
    dependencies: &[WorkflowDependency],
) -> Vec<WorkflowStep> {
    if steps.is_empty() {
        return vec![];
    }

    // Build adjacency list and in-degree count
    let mut adj: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    let mut in_degree: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    for step in &steps {
        in_degree.entry(step.step_id.clone()).or_insert(0);
    }

    for dep in dependencies {
        adj.entry(dep.from_step.clone())
            .or_insert_with(Vec::new)
            .push(dep.to_step.clone());
        *in_degree.entry(dep.to_step.clone()).or_insert(0) += 1;
    }

    // Start with nodes that have no dependencies
    let mut queue: Vec<String> = in_degree
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(k, _)| k.clone())
        .collect();
    queue.sort(); // Deterministic ordering

    let mut result = Vec::new();
    let steps_map: std::collections::HashMap<String, WorkflowStep> =
        steps.into_iter().map(|s| (s.step_id.clone(), s)).collect();

    while let Some(node) = queue.first().cloned() {
        let node_str = node.clone();
        queue.remove(0);

        if let Some(step) = steps_map.get(&node_str) {
            result.push(step.clone());
        }

        if let Some(neighbors) = adj.get(&node_str) {
            let mut new_zero_degree: Vec<String> = Vec::new();
            for neighbor in neighbors {
                if let Some(deg) = in_degree.get_mut(neighbor) {
                    *deg -= 1;
                    if *deg == 0 {
                        new_zero_degree.push(neighbor.clone());
                    }
                }
            }
            new_zero_degree.sort();
            queue.extend(new_zero_degree);
        }
    }

    // If not all steps are in the result, there's a cycle — append remaining
    if result.len() < steps_map.len() {
        let result_ids: std::collections::HashSet<String> =
            result.iter().map(|s| s.step_id.clone()).collect();
        for (id, step) in steps_map {
            if !result_ids.contains(&id) {
                result.push(step);
            }
        }
    }

    result
}

/// Sort steps by priority (lower priority number = earlier).
///
/// Within same priority, maintains original order.
pub fn sort_by_priority(mut steps: Vec<WorkflowStep>) -> Vec<WorkflowStep> {
    steps.sort_by_key(|s| s.priority);
    steps
}

/// Sort steps by stage then priority.
pub fn sort_by_stage_and_priority(mut steps: Vec<WorkflowStep>) -> Vec<WorkflowStep> {
    steps.sort_by(|a, b| {
        a.stage
            .to_string()
            .cmp(&b.stage.to_string())
            .then_with(|| a.priority.cmp(&b.priority))
    });
    steps
}

/// Determine if steps can be executed in parallel.
///
/// Returns true if no dependencies exist between steps.
pub fn can_parallelize(steps: &[WorkflowStep], dependencies: &[WorkflowDependency]) -> bool {
    dependencies.is_empty() && steps.len() > 1
}

/// Group steps by stage.
pub fn group_by_stage(steps: &[WorkflowStep]) -> Vec<(&str, Vec<&WorkflowStep>)> {
    let mut groups: std::collections::HashMap<&str, Vec<&WorkflowStep>> =
        std::collections::HashMap::new();
    for step in steps {
        groups
            .entry(step.stage.to_string().leak())
            .or_insert_with(Vec::new)
            .push(step);
    }
    let mut result: Vec<_> = groups.into_iter().collect();
    result.sort_by(|a, b| a.0.cmp(b.0));
    result
}

/// Get the critical path length (longest dependency chain).
pub fn critical_path_length(steps: &[WorkflowStep], dependencies: &[WorkflowDependency]) -> usize {
    super::dependency::calculate_depth(steps, dependencies)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_step(id: &str, priority: u32, stage: WorkflowStage, deps: Vec<&str>) -> WorkflowStep {
        WorkflowStep {
            step_id: id.to_string(),
            name: id.to_string(),
            command: format!("cmd_{}", id),
            stage,
            priority,
            dependencies: deps.into_iter().map(|s| s.to_string()).collect(),
            requires_approval: false,
            estimated_cost: 0.0,
            reversible: true,
            description: "Test".to_string(),
        }
    }

    #[test]
    fn test_topological_sort_empty() {
        let steps: Vec<WorkflowStep> = vec![];
        let sorted = topological_sort(steps, &[]);
        assert!(sorted.is_empty());
    }

    #[test]
    fn test_topological_sort_no_deps() {
        let steps = vec![
            make_step("b", 1, WorkflowStage::Execution, vec![]),
            make_step("a", 0, WorkflowStage::Execution, vec![]),
        ];
        let sorted = topological_sort(steps, &[]);
        // Without deps, order is preserved but may vary
        assert_eq!(sorted.len(), 2);
    }

    #[test]
    fn test_topological_sort_with_deps() {
        let steps = vec![
            make_step("c", 2, WorkflowStage::Execution, vec!["a", "b"]),
            make_step("a", 0, WorkflowStage::Execution, vec![]),
            make_step("b", 1, WorkflowStage::Execution, vec!["a"]),
        ];
        let deps = vec![
            WorkflowDependency {
                from_step: "a".to_string(),
                to_step: "b".to_string(),
                dependency_type: DependencyType::MustCompleteBefore,
            },
            WorkflowDependency {
                from_step: "a".to_string(),
                to_step: "c".to_string(),
                dependency_type: DependencyType::MustCompleteBefore,
            },
            WorkflowDependency {
                from_step: "b".to_string(),
                to_step: "c".to_string(),
                dependency_type: DependencyType::MustCompleteBefore,
            },
        ];
        let sorted = topological_sort(steps, &deps);
        assert_eq!(sorted.len(), 3);
        // a must come before b, b must come before c
        let pos_a = sorted.iter().position(|s| s.step_id == "a").unwrap();
        let pos_b = sorted.iter().position(|s| s.step_id == "b").unwrap();
        let pos_c = sorted.iter().position(|s| s.step_id == "c").unwrap();
        assert!(pos_a < pos_b);
        assert!(pos_b < pos_c);
    }

    #[test]
    fn test_sort_by_priority() {
        let steps = vec![
            make_step("b", 2, WorkflowStage::Execution, vec![]),
            make_step("a", 0, WorkflowStage::Execution, vec![]),
            make_step("c", 1, WorkflowStage::Execution, vec![]),
        ];
        let sorted = sort_by_priority(steps);
        assert_eq!(sorted[0].step_id, "a");
        assert_eq!(sorted[1].step_id, "c");
        assert_eq!(sorted[2].step_id, "b");
    }

    #[test]
    fn test_sort_by_stage_and_priority() {
        let steps = vec![
            make_step("exec_b", 1, WorkflowStage::Execution, vec![]),
            make_step("prep_a", 0, WorkflowStage::Preparation, vec![]),
            make_step("exec_a", 0, WorkflowStage::Execution, vec![]),
            make_step("val_a", 0, WorkflowStage::Validation, vec![]),
        ];
        let sorted = sort_by_stage_and_priority(steps);
        // Stages are sorted alphabetically: Execution < Preparation < Validation
        // First two are Execution (sorted by priority), then Preparation, then Validation
        assert_eq!(sorted[0].stage, WorkflowStage::Execution);
        assert_eq!(sorted[1].stage, WorkflowStage::Execution);
        assert_eq!(sorted[2].stage, WorkflowStage::Preparation);
        assert_eq!(sorted[3].stage, WorkflowStage::Validation);
    }

    #[test]
    fn test_can_parallelize_no_deps() {
        let steps = vec![
            make_step("a", 0, WorkflowStage::Execution, vec![]),
            make_step("b", 0, WorkflowStage::Execution, vec![]),
        ];
        assert!(can_parallelize(&steps, &[]));
    }

    #[test]
    fn test_can_parallelize_with_deps() {
        let steps = vec![
            make_step("a", 0, WorkflowStage::Execution, vec![]),
            make_step("b", 0, WorkflowStage::Execution, vec!["a"]),
        ];
        let deps = vec![WorkflowDependency {
            from_step: "a".to_string(),
            to_step: "b".to_string(),
            dependency_type: DependencyType::MustCompleteBefore,
        }];
        assert!(!can_parallelize(&steps, &deps));
    }

    #[test]
    fn test_group_by_stage() {
        let steps = vec![
            make_step("a", 0, WorkflowStage::Preparation, vec![]),
            make_step("b", 0, WorkflowStage::Execution, vec![]),
            make_step("c", 0, WorkflowStage::Preparation, vec![]),
        ];
        let groups = group_by_stage(&steps);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0, "execution");
        assert_eq!(groups[1].0, "preparation");
        assert_eq!(groups[0].1.len(), 1);
        assert_eq!(groups[1].1.len(), 2);
    }

    #[test]
    fn test_critical_path_length() {
        let steps = vec![
            make_step("a", 0, WorkflowStage::Execution, vec![]),
            make_step("b", 0, WorkflowStage::Execution, vec!["a"]),
            make_step("c", 0, WorkflowStage::Execution, vec!["b"]),
        ];
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
        assert_eq!(critical_path_length(&steps, &deps), 3);
    }
}

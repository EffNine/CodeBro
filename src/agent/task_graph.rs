#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskNode {
    pub id: String,
    pub description: String,
    pub agent: String,
    pub status: TaskStatus,
    pub dependencies: Vec<String>,
    pub result: Option<String>,
    pub duration_ms: u64,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskStatus::Pending => write!(f, "pending"),
            TaskStatus::Running => write!(f, "running"),
            TaskStatus::Completed => write!(f, "completed"),
            TaskStatus::Failed => write!(f, "failed"),
            TaskStatus::Skipped => write!(f, "skipped"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskGraph {
    pub nodes: HashMap<String, TaskNode>,
    pub edges: Vec<(String, String)>,
    pub root_task: String,
}

impl TaskGraph {
    pub fn new(root_description: &str) -> Self {
        let root_id = uuid::Uuid::new_v4().to_string();
        let root_node = TaskNode {
            id: root_id.clone(),
            description: root_description.to_string(),
            agent: "main".to_string(),
            status: TaskStatus::Pending,
            dependencies: Vec::new(),
            result: None,
            duration_ms: 0,
            created_at: chrono::Local::now().to_rfc3339(),
            started_at: None,
            completed_at: None,
        };

        TaskGraph {
            nodes: {
                let mut nodes = HashMap::new();
                nodes.insert(root_id.clone(), root_node);
                nodes
            },
            edges: Vec::new(),
            root_task: root_id,
        }
    }

    pub fn add_task(
        &mut self,
        description: &str,
        agent: &str,
        dependencies: Vec<String>,
    ) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let node = TaskNode {
            id: id.clone(),
            description: description.to_string(),
            agent: agent.to_string(),
            status: TaskStatus::Pending,
            dependencies,
            result: None,
            duration_ms: 0,
            created_at: chrono::Local::now().to_rfc3339(),
            started_at: None,
            completed_at: None,
        };

        self.nodes.insert(id.clone(), node);
        id
    }

    pub fn add_edge(&mut self, from: &str, to: &str) {
        self.edges.push((from.to_string(), to.to_string()));
    }

    pub fn get_task(&self, id: &str) -> Option<&TaskNode> {
        self.nodes.get(id)
    }

    pub fn get_task_mut(&mut self, id: &str) -> Option<&mut TaskNode> {
        self.nodes.get_mut(id)
    }

    pub fn update_status(&mut self, id: &str, status: TaskStatus) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.status = status.clone();
            match status {
                TaskStatus::Running => {
                    node.started_at = Some(chrono::Local::now().to_rfc3339());
                }
                TaskStatus::Completed | TaskStatus::Failed => {
                    node.completed_at = Some(chrono::Local::now().to_rfc3339());
                }
                _ => {}
            }
        }
    }

    pub fn set_result(&mut self, id: &str, result: &str) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.result = Some(result.to_string());
        }
    }

    pub fn get_ready_tasks(&self) -> Vec<&TaskNode> {
        self.nodes
            .values()
            .filter(|node| {
                node.id != self.root_task
                    && node.status == TaskStatus::Pending
                    && node.dependencies.iter().all(|dep| {
                        self.nodes
                            .get(dep)
                            .map(|n| n.status == TaskStatus::Completed)
                            .unwrap_or(false)
                    })
            })
            .collect()
    }

    pub fn get_running_tasks(&self) -> Vec<&TaskNode> {
        self.nodes
            .values()
            .filter(|node| node.status == TaskStatus::Running)
            .collect()
    }

    pub fn get_failed_tasks(&self) -> Vec<&TaskNode> {
        self.nodes
            .values()
            .filter(|node| node.status == TaskStatus::Failed)
            .collect()
    }

    pub fn is_complete(&self) -> bool {
        self.nodes.values().all(|node| {
            node.id == self.root_task
                || node.status == TaskStatus::Completed
                || node.status == TaskStatus::Skipped
        })
    }

    pub fn has_failures(&self) -> bool {
        self.nodes
            .values()
            .any(|node| node.status == TaskStatus::Failed)
    }

    pub fn execution_order(&self) -> Vec<String> {
        let mut order = Vec::new();
        let mut visited = std::collections::HashSet::new();
        let mut temp_visited = std::collections::HashSet::new();

        fn visit(
            id: &str,
            graph: &TaskGraph,
            visited: &mut std::collections::HashSet<String>,
            temp_visited: &mut std::collections::HashSet<String>,
            order: &mut Vec<String>,
        ) {
            if visited.contains(id) {
                return;
            }
            if temp_visited.contains(id) {
                return;
            }

            temp_visited.insert(id.to_string());

            if let Some(node) = graph.nodes.get(id) {
                for dep in &node.dependencies {
                    visit(dep, graph, visited, temp_visited, order);
                }
            }

            temp_visited.remove(id);
            visited.insert(id.to_string());
            order.push(id.to_string());
        }

        for id in self.nodes.keys() {
            visit(id, self, &mut visited, &mut temp_visited, &mut order);
        }

        order
    }

    pub fn summary(&self) -> String {
        let total = self.nodes.len();
        let completed = self
            .nodes
            .values()
            .filter(|n| n.status == TaskStatus::Completed)
            .count();
        let failed = self
            .nodes
            .values()
            .filter(|n| n.status == TaskStatus::Failed)
            .count();
        let pending = self
            .nodes
            .values()
            .filter(|n| n.status == TaskStatus::Pending)
            .count();
        let running = self
            .nodes
            .values()
            .filter(|n| n.status == TaskStatus::Running)
            .count();

        format!(
            "Task Graph: {} total, {} completed, {} failed, {} pending, {} running",
            total, completed, failed, pending, running
        )
    }
}

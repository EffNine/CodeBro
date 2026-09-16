//! Tiny task registry (eval fixture). Steps 1-2 to be implemented in
//! session A, steps 3-4 in session B. See `spec.md`.
use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub struct TaskRegistry {
    tasks: HashMap<String, String>,
}

impl TaskRegistry {
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
        }
    }

    // --- Step 1 ---
    pub fn add(&mut self, id: &str, title: &str) -> Result<(), String> {
        if id.is_empty() {
            return Err("id must not be empty".to_string());
        }
        if title.is_empty() {
            return Err("title must not be empty".to_string());
        }
        if self.tasks.contains_key(id) {
            return Err(format!("id already exists: {id}"));
        }
        self.tasks.insert(id.to_string(), title.to_string());
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<String> {
        self.tasks.get(id).cloned()
    }

    // --- Step 2 ---
    pub fn list(&self) -> Vec<(String, String)> {
        let mut pairs: Vec<(String, String)> =
            self.tasks.iter().map(|(id, t)| (id.clone(), t.clone())).collect();
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        pairs
    }

    pub fn remove(&mut self, id: &str) -> bool {
        self.tasks.remove(id).is_some()
    }

    // --- Step 3 ---
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut pairs: Vec<(String, String)> =
            self.tasks.iter().map(|(id, t)| (id.clone(), t.clone())).collect();
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        let mut content = String::new();
        for (id, title) in &pairs {
            content.push_str(id);
            content.push(':');
            content.push_str(title);
            content.push('\n');
        }
        fs::write(path, content)
    }

    pub fn load(path: &Path) -> std::io::Result<Self> {
        let data = fs::read_to_string(path)?;
        let mut tasks = HashMap::new();
        for line in data.lines() {
            let line = line.trim_end_matches('\n');
            if line.is_empty() {
                continue;
            }
            let (id, title) = line
                .split_once(':')
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "malformed line: missing ':' separator"))?;
            tasks.insert(id.to_string(), title.to_string());
        }
        Ok(Self { tasks })
    }

    // --- Step 4 ---
    pub fn rename(&mut self, id: &str, new_title: &str) -> Result<(), String> {
        if new_title.is_empty() {
            return Err("new_title must not be empty".to_string());
        }
        match self.tasks.get_mut(id) {
            Some(title) => {
                *title = new_title.to_string();
                Ok(())
            }
            None => Err(format!("unknown id: {id}")),
        }
    }
}

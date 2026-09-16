//! Tiny task registry (eval fixture). Steps 1-2 to be implemented in
//! session A, steps 3-4 in session B. See `spec.md`.
use std::collections::HashMap;
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
            return Err(format!("id '{}' already exists", id));
        }
        self.tasks.insert(id.to_string(), title.to_string());
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<String> {
        self.tasks.get(id).cloned()
    }

    // --- Step 2 ---
    pub fn list(&self) -> Vec<(String, String)> {
        let mut entries: Vec<(String, String)> =
            self.tasks.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
    }

    pub fn remove(&mut self, id: &str) -> bool {
        self.tasks.remove(id).is_some()
    }

    // --- Step 3 ---
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut s = String::new();
        for (id, title) in self.list() {
            s.push_str(&format!("{}:{}\n", id, title));
        }
        std::fs::write(path, s)
    }

    pub fn load(path: &Path) -> std::io::Result<Self> {
        let s = std::fs::read_to_string(path)?;
        let mut r = Self::new();
        for line in s.lines() {
            match line.split_once(':') {
                Some((id, title)) => {
                    r.tasks.insert(id.to_string(), title.to_string());
                }
                None => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("malformed line (missing ':'): {:?}", line),
                    ));
                }
            }
        }
        Ok(r)
    }

    // --- Step 4 ---
    pub fn rename(&mut self, id: &str, new_title: &str) -> Result<(), String> {
        if new_title.is_empty() {
            return Err("new title must not be empty".to_string());
        }
        match self.tasks.get_mut(id) {
            Some(title) => {
                *title = new_title.to_string();
                Ok(())
            }
            None => Err(format!("id '{}' not found", id)),
        }
    }
}

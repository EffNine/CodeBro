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
            return Err("id must not be empty".into());
        }
        if title.is_empty() {
            return Err("title must not be empty".into());
        }
        if self.tasks.contains_key(id) {
            return Err(format!("duplicate id: {id}"));
        }
        self.tasks.insert(id.to_string(), title.to_string());
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<String> {
        self.tasks.get(id).cloned()
    }

    // --- Step 2 ---
    pub fn list(&self) -> Vec<(String, String)> {
        let mut items: Vec<(String, String)> =
            self.tasks.iter().map(|(id, t)| (id.clone(), t.clone())).collect();
        items.sort_by(|a, b| a.0.cmp(&b.0));
        items
    }

    pub fn remove(&mut self, id: &str) -> bool {
        self.tasks.remove(id).is_some()
    }

    // --- Step 3 ---
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut out = String::new();
        for (id, title) in self.list() {
            out.push_str(&format!("{id}:{title}\n"));
        }
        std::fs::write(path, out)
    }

    pub fn load(path: &Path) -> std::io::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let mut reg = Self::new();
        for line in content.lines() {
            if line.is_empty() {
                continue;
            }
            let (id, title) = line.split_once(':').ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "malformed line")
            })?;
            reg.tasks.insert(id.to_string(), title.to_string());
        }
        Ok(reg)
    }

    // --- Step 4 ---
    pub fn rename(&mut self, id: &str, new_title: &str) -> Result<(), String> {
        if new_title.is_empty() {
            return Err("new_title must not be empty".into());
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

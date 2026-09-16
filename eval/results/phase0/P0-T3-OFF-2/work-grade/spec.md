# `TaskRegistry` specification (fixture crate `registry`)

An in-memory registry of tasks, each with a string `id` and a string `title`,
with optional persistence to a text file. Standard library only.

## API

```rust
pub struct TaskRegistry { /* private */ }

impl TaskRegistry {
    pub fn new() -> Self;

    // --- Step 1: basic storage ---
    pub fn add(&mut self, id: &str, title: &str) -> Result<(), String>;
    pub fn get(&self, id: &str) -> Option<String>;

    // --- Step 2: listing and removal ---
    pub fn list(&self) -> Vec<(String, String)>;
    pub fn remove(&mut self, id: &str) -> bool;

    // --- Step 3: persistence ---
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()>;
    pub fn load(path: &std::path::Path) -> std::io::Result<Self>;

    // --- Step 4: rename + validation ---
    pub fn rename(&mut self, id: &str, new_title: &str) -> Result<(), String>;
}
```

## Rules

- `add` fails (`Err`) when `id` is empty, when `title` is empty, or when `id`
  already exists. Otherwise stores the entry.
- `get` returns a clone of the title, or `None` when `id` is unknown.
- `list` returns all `(id, title)` pairs sorted by `id` ascending.
- `remove` deletes the entry, returning `true` when something was removed and
  `false` when `id` was unknown.
- `save` writes one line per entry, sorted by `id`, in the exact format
  `id:title\n`. It creates or truncates the file.
- `load` reads a file written by `save` and reconstructs the registry.
  A missing file is an `Err`. A malformed line (no `:` separator) is an `Err`.
  Split each line at the FIRST `:` only.
- `rename` fails (`Err`) when `id` is unknown or when `new_title` is empty.
  Otherwise updates the title in place.
- Tests never use `:` or newline characters inside ids or titles.

## Steps

1. `new` + `add` + `get`.
2. `list` + `remove`.
3. `save` + `load`.
4. `rename` (+ the validation rules above for `add`/`rename`).

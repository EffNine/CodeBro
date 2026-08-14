//! CodeBro TUI Handler — simplified request dispatcher
//! Maps TUI commands to CodeBro backend operations via stdio JSON protocol

use super::protocol::{TuiRequest, TuiResponse};
use super::bridge::TuiState;
use anyhow::Result;
use serde_json::json;
use std::sync::Mutex;

// ─── Simple global state ─────────────────────────────────────────────────────

struct AppState {
    config_dir: std::path::PathBuf,
    current_session: Mutex<Option<String>>,
}

impl AppState {
    fn new() -> Result<Self> {
        Ok(AppState {
            config_dir: crate::config::Config::config_dir(),
            current_session: Mutex::new(None),
        })
    }

    fn session_store(&self) -> Result<crate::session::SessionStore> {
        Ok(crate::session::SessionStore::new(&self.config_dir)?)
    }

    fn session_tracker(&self) -> Result<crate::session::SessionTracker> {
        Ok(crate::session::SessionTracker::new(&self.config_dir)?)
    }

    fn provider_manager(&self) -> Result<crate::provider_manager::ProviderManager> {
        let mut pm = crate::provider_manager::ProviderManager::new(self.config_dir.clone());
        pm.register_builtin();
        let _ = pm.load();
        Ok(pm)
    }
}

static APP_STATE: std::sync::OnceLock<Mutex<AppState>> = std::sync::OnceLock::new();

fn with_state<F, R>(f: F) -> Result<R>
where
    F: FnOnce(&mut AppState) -> Result<R>,
{
    let state = APP_STATE.get_or_init(|| {
        Mutex::new(AppState::new().expect("Failed to init app state"))
    });
    let mut guard = state.lock().unwrap();
    f(&mut guard)
}

// ─── Request Handler ─────────────────────────────────────────────────────────

pub async fn handle_request(req: TuiRequest, _state: &TuiState) -> TuiResponse {
    let id = req.id;
    let cmd = req.cmd.as_str();
    let payload = req.payload.unwrap_or_default();

    let result = match cmd {
        "session.list" => handle_session_list(),
        "session.get" => handle_session_get(&payload),
        "session.create" => handle_session_create(&payload),
        "session.delete" => handle_session_delete(&payload),
        "session.abort" => Ok(json!({})),
        "session.status" => Ok(json!("idle")),
        "session.messages" => handle_session_messages(&payload),
        "session.diff" => Ok(json!([])),
        "session.todo" => Ok(json!([])),
        "session.fork" => handle_session_get(&payload),
        "session.revert" => Ok(json!({})),
        "session.unrevert" => Ok(json!({})),
        "session.summarize" => Ok(json!({})),
        "session.shell" => handle_session_shell(&payload).await,
        "session.command" => handle_session_command(&payload, _state).await,
        "session.children" => Ok(json!([])),
        "session.permission" => Ok(json!([])),
        "session.question" => Ok(json!([])),
        "session.update" => handle_session_get(&payload),
        "provider.list" => handle_provider_list(),
        "provider.auth" => handle_provider_auth(&payload),
        "permission.reply" => Ok(json!({})),
        "question.reply" => Ok(json!({})),
        "question.reject" => Ok(json!({})),
        "mcp.status" => Ok(json!({})),
        "mcp.connect" => Ok(json!({})),
        "mcp.disconnect" => Ok(json!({})),
        "project.current" => handle_project_current(),
        "project.directories" => Ok(json!([])),
        "path.get" => Ok(json!({
            "home": dirs::home_dir().map(|d| d.to_string_lossy().to_string()).unwrap_or_default(),
            "state": format!("{}/.state", dirs::home_dir().map(|d| d.to_string_lossy().to_string()).unwrap_or_default()),
            "config": format!("{}/.codebro", dirs::home_dir().map(|d| d.to_string_lossy().to_string()).unwrap_or_default()),
            "worktree": "",
            "directory": std::env::current_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
        })),
        "vcs.info" => Ok(json!({})),
        "vcs.status" => Ok(json!([])),
        "vcs.get" => Ok(json!({})),
        "lsp.status" => Ok(json!([])),
        "pty.create" => handle_pty_create(&payload),
        "pty.list" => Ok(json!([])),
        "pty.get" => Ok(json!({})),
        "pty.remove" => Ok(json!({})),
        "command.list" => handle_command_list(),
        "file.list" => handle_file_list(&payload),
        "file.read" => handle_file_read(&payload),
        "file.status" => Ok(json!({})),
        "find.files" => handle_find_files(&payload),
        "find.symbols" => Ok(json!([])),
        "find.text" => Ok(json!([])),
        "auth.set" => handle_auth_set(&payload),
        "auth.remove" => handle_auth_remove(&payload),
        "config.get" => handle_config_get(),
        "config.providers" => handle_provider_list(),
        "app.agents" => handle_app_agents(),
        "global.health" => Ok(json!({ "status": "ok" })),
        "global.dispose" => Ok(json!({})),
        "global.upgrade" => Ok(json!({})),
        "instance.dispose" => Ok(json!({})),
        "formatter.status" => Ok(json!([])),
        "tui.prompt.append" => Ok(json!({})),
        "tui.command.execute" => Ok(json!({})),
        "tui.session.select" => Ok(json!({})),
        "tui.toast.show" => Ok(json!({})),
        _ => Err(anyhow::anyhow!("Unknown command: {}", cmd)),
    };

    match result {
        Ok(data) => TuiResponse {
            id,
            result: Some(json!({ "data": data })),
            error: None,
        },
        Err(e) => TuiResponse {
            id,
            result: None,
            error: Some(e.to_string()),
        },
    }
}

// ─── Session Handlers ────────────────────────────────────────────────────────

fn handle_session_list() -> Result<serde_json::Value> {
    with_state(|s| {
        let store = s.session_store()?;
        let sessions = store.list_sessions()?;
        Ok(json!(sessions.iter().map(|sess| {
            let title = if sess.task.is_empty() { "New Session" } else { &sess.task[..sess.task.len().min(40)] };
            json!({
                "id": sess.id,
                "slug": &sess.id[..8.min(sess.id.len())],
                "projectID": "",
                "directory": ".",
                "title": title,
                "agent": "main",
                "model": null,
                "time": { "created": 0, "updated": 0 },
                "status": "idle",
            })
        }).collect::<Vec<_>>()))
    })
}

fn handle_session_get(payload: &serde_json::Value) -> Result<serde_json::Value> {
    let id = payload.get("id").and_then(|v| v.as_str()).unwrap_or("");
    if id.is_empty() {
        return Ok(json!({
            "id": "new", "slug": "new", "projectID": "", "directory": ".",
            "title": "New Session", "time": { "created": 0, "updated": 0 }, "status": "idle",
        }));
    }
    with_state(|s| {
        let store = s.session_store()?;
        match store.load_session(id) {
            Ok(session) => {
                let title = if session.task.is_empty() { "New Session" } else { &session.task[..session.task.len().min(40)] };
                Ok(json!({
                    "id": session.id,
                    "slug": &session.id[..8.min(session.id.len())],
                    "projectID": "", "directory": ".",
                    "title": title, "agent": "main", "model": null,
                    "time": { "created": 0, "updated": 0 }, "status": "idle",
                }))
            }
            Err(_) => Ok(json!({
                "id": id, "slug": &id[..8.min(id.len())],
                "projectID": "", "directory": ".", "title": "Session",
                "time": { "created": 0, "updated": 0 }, "status": "idle",
            })),
        }
    })
}

fn handle_session_create(payload: &serde_json::Value) -> Result<serde_json::Value> {
    let task = payload.get("task").and_then(|v| v.as_str()).unwrap_or("").to_string();
    with_state(|s| {
        let mut tracker = s.session_tracker()?;
        let id = tracker.start_session(&task)?;
        *s.current_session.lock().unwrap() = Some(id.clone());
        let title = if task.is_empty() { "New Session" } else { &task[..task.len().min(40)] };
        Ok(json!({
            "id": id,
            "slug": &id[..8.min(id.len())],
            "projectID": "", "directory": ".",
            "title": title,
            "time": { "created": chrono::Utc::now().timestamp_millis(), "updated": chrono::Utc::now().timestamp_millis() },
            "status": "idle",
        }))
    })
}

fn handle_session_delete(payload: &serde_json::Value) -> Result<serde_json::Value> {
    let id = payload.get("id").and_then(|v| v.as_str()).unwrap_or("");
    if id.is_empty() { return Ok(json!({})); }
    with_state(|s| {
        let _ = s.session_store()?.delete_session(id);
        if let Some(current) = s.current_session.lock().unwrap().as_ref() {
            if current == id { *s.current_session.lock().unwrap() = None; }
        }
        Ok(json!({}))
    })
}

fn handle_session_messages(payload: &serde_json::Value) -> Result<serde_json::Value> {
    let id = payload.get("id").and_then(|v| v.as_str()).unwrap_or("");
    if id.is_empty() { return Ok(json!([])); }
    with_state(|s| {
        match s.session_store()?.load_session(id) {
            Ok(session) => {
                let mut messages = Vec::new();
                if !session.task.is_empty() {
                    messages.push(json!({
                        "id": format!("user-{}", id), "sessionID": id, "role": "user",
                        "content": session.task, "parts": [], "time": { "created": 0 },
                    }));
                }
                if let Some(result) = &session.result {
                    messages.push(json!({
                        "id": format!("assistant-{}", id), "sessionID": id, "role": "assistant",
                        "content": result, "parts": [], "time": { "created": 0 },
                    }));
                }
                Ok(json!(messages))
            }
            Err(_) => Ok(json!([])),
        }
    })
}

async fn handle_session_shell(payload: &serde_json::Value) -> Result<serde_json::Value> {
    let command = payload.get("command").and_then(|v| v.as_str()).unwrap_or("echo hello");
    let output = tokio::task::block_in_place(|| {
        std::process::Command::new("sh").arg("-c").arg(command).output()
    });
    match output {
        Ok(out) => Ok(json!({
            "id": "shell", "pid": 0,
            "output": String::from_utf8_lossy(&out.stdout).to_string(),
            "error": String::from_utf8_lossy(&out.stderr).to_string(),
            "exit_code": out.status.code().unwrap_or(1),
        })),
        Err(e) => Ok(json!({ "error": e.to_string(), "exit_code": 127 })),
    }
}

async fn handle_session_command(payload: &serde_json::Value, _state: &TuiState) -> Result<serde_json::Value> {
    let session_id = payload.get("sessionID")
        .or(payload.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let text = payload.get("text")
        .or(payload.get("command"))
        .or(payload.get("task"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if text.is_empty() {
        return Ok(json!({ "id": session_id, "status": "idle" }));
    }

    with_state(|s| {
        let session_id = if session_id.is_empty() || session_id == "new" {
            let mut tracker = s.session_tracker()?;
            tracker.start_session(text)?
        } else {
            session_id.to_string()
        };
        *s.current_session.lock().unwrap() = Some(session_id.clone());

        let response = format!("[CodeBro] Received: {}\n\nThis is a stub response. Full CanonicalRuntime integration is in progress.", text);

        if let Ok(mut tracker) = s.session_tracker() {
            if let Some(sess) = tracker.current_session_mut() {
                sess.set_result(&response);
            }
            let _ = tracker.store().save_session(
                &tracker.current_session().expect("session just created").clone(),
            );
        }

        Ok(json!({
            "id": session_id,
            "status": "completed",
            "response": response,
        }))
    })
}

// ─── Provider Handlers ────────────────────────────────────────────────────────

fn handle_provider_list() -> Result<serde_json::Value> {
    with_state(|s| {
        let pm = s.provider_manager()?;
        Ok(json!(pm.list_providers_ordered().iter().map(|(id, entry)| {
            json!({
                "id": id,
                "name": entry.id.to_string(),
                "baseURL": entry.base_url.clone(),
                "models": Vec::<serde_json::Value>::new(),
                "health": match &entry.health {
                    crate::provider_manager::HealthStatus::Healthy => "connected",
                    crate::provider_manager::HealthStatus::Unhealthy { .. } => "error",
                    _ => "disconnected",
                },
                "apiKeySet": entry.api_key.is_some(),
                "currentModel": entry.current_model.clone(),
            })
        }).collect::<Vec<_>>()))
    })
}

fn handle_provider_auth(payload: &serde_json::Value) -> Result<serde_json::Value> {
    let provider_id = payload.get("providerID").and_then(|v| v.as_str()).unwrap_or("");
    let api_key = payload.get("apiKey").and_then(|v| v.as_str()).unwrap_or("");
    if provider_id.is_empty() || api_key.is_empty() { return Ok(json!({})); }
    with_state(|s| {
        let mut pm = s.provider_manager()?;
        pm.set_api_key(provider_id, api_key)?;
        let _ = pm.persist();
        Ok(json!({ "status": "ok" }))
    })
}

// ─── File/Find Handlers ───────────────────────────────────────────────────────

fn handle_file_list(payload: &serde_json::Value) -> Result<serde_json::Value> {
    let cwd = payload.get("cwd").and_then(|v| v.as_str()).unwrap_or(".");
    let mut entries = Vec::new();
    for entry in walkdir::WalkDir::new(cwd).max_depth(1).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_file() {
            entries.push(json!({
                "path": path.to_string_lossy().to_string(),
                "name": path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                "type": "file",
            }));
        }
    }
    Ok(json!(entries))
}

fn handle_file_read(payload: &serde_json::Value) -> Result<serde_json::Value> {
    let path = payload.get("path").and_then(|v| v.as_str()).unwrap_or("");
    Ok(json!(std::fs::read_to_string(path).unwrap_or_default()))
}

fn handle_find_files(payload: &serde_json::Value) -> Result<serde_json::Value> {
    let pattern = payload.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
    let cwd = payload.get("cwd").and_then(|v| v.as_str()).unwrap_or(".");
    let mut results = Vec::new();
    for entry in walkdir::WalkDir::new(cwd).max_depth(2).into_iter().filter_map(|e| e.ok()) {
        if entry.path().is_file() {
            let p = entry.path().to_string_lossy().to_string();
            if pattern.is_empty() || p.contains(pattern) {
                results.push(p);
            }
        }
    }
    Ok(json!(results))
}

// ─── PTY Handler ──────────────────────────────────────────────────────────────

fn handle_pty_create(payload: &serde_json::Value) -> Result<serde_json::Value> {
    let command = payload.get("command").and_then(|v| v.as_str()).unwrap_or("bash");
    let output = tokio::task::block_in_place(|| {
        std::process::Command::new("sh").arg("-c").arg(command).output()
    });
    match output {
        Ok(out) => Ok(json!({
            "id": format!("pty-{}", uuid::Uuid::new_v4()),
            "pid": 0,
            "output": String::from_utf8_lossy(&out.stdout).to_string(),
        })),
        Err(e) => Ok(json!({ "error": e.to_string() })),
    }
}

// ─── Config Handlers ──────────────────────────────────────────────────────────

fn handle_project_current() -> Result<serde_json::Value> {
    let dir = std::env::current_dir().unwrap_or_default();
    Ok(json!({ "directory": dir.to_string_lossy().to_string(), "name": "CodeBro" }))
}

fn handle_command_list() -> Result<serde_json::Value> {
    Ok(json!([
        { "id": "chat", "name": "chat", "description": "Chat with CodeBro" },
        { "id": "research", "name": "research", "description": "Research subagent" },
        { "id": "plan", "name": "plan", "description": "Planning subagent" },
        { "id": "code", "name": "code", "description": "Coding subagent" },
        { "id": "test", "name": "test", "description": "Testing subagent" },
        { "id": "review", "name": "review", "description": "Review subagent" },
    ]))
}

fn handle_auth_set(payload: &serde_json::Value) -> Result<serde_json::Value> {
    let provider = payload.get("provider").and_then(|v| v.as_str()).unwrap_or("");
    let api_key = payload.get("auth").and_then(|v| v.get("key")).and_then(|v| v.as_str()).unwrap_or("");
    if provider.is_empty() || api_key.is_empty() { return Ok(json!({})); }
    with_state(|s| {
        let mut pm = s.provider_manager()?;
        pm.set_api_key(provider, api_key)?;
        let _ = pm.persist();
        Ok(json!({ "status": "ok" }))
    })
}

fn handle_auth_remove(payload: &serde_json::Value) -> Result<serde_json::Value> {
    let provider = payload.get("provider").and_then(|v| v.as_str()).unwrap_or("");
    if provider.is_empty() { return Ok(json!({})); }
    with_state(|s| {
        let mut pm = s.provider_manager()?;
        let _ = pm.clear_api_key(provider);
        let _ = pm.persist();
        Ok(json!({ "status": "ok" }))
    })
}

fn handle_config_get() -> Result<serde_json::Value> {
    let config = match crate::config::Config::load() {
        Ok(c) => c,
        Err(_) => crate::config::Config {
            provider: "openai".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            model: String::new(),
            api_key: None,
        },
    };
    Ok(json!({
        "provider": config.provider,
        "baseUrl": config.base_url,
        "model": config.model,
        "theme": "dark",
        "mouse": true,
    }))
}

// ─── App Handlers ─────────────────────────────────────────────────────────────

fn handle_app_agents() -> Result<serde_json::Value> {
    Ok(json!([
        { "id": "main", "name": "Main", "description": "Main agent" },
        { "id": "research", "name": "Research", "description": "Research subagent" },
        { "id": "planning", "name": "Planning", "description": "Planning subagent" },
        { "id": "coding", "name": "Coding", "description": "Coding subagent" },
        { "id": "testing", "name": "Testing", "description": "Testing subagent" },
        { "id": "review", "name": "Review", "description": "Review subagent" },
    ]))
}

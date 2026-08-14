//! CodeBro TUI Handler — simplified request dispatcher
//! Maps TUI commands to CodeBro backend operations via stdio JSON protocol

use super::bridge::TuiState;
use super::protocol::{TuiEvent, TuiRequest, TuiResponse};
use anyhow::Result;
use serde_json::json;
use std::sync::{Arc, Mutex};

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

    fn build_runtime(&self) -> Result<crate::canonical_runtime::CanonicalRuntime> {
        let config = crate::config::Config::load()?;
        let rt = crate::canonical_runtime::CanonicalRuntime::new(config)?;
        Ok(rt)
    }
}

static APP_STATE: std::sync::OnceLock<Mutex<AppState>> = std::sync::OnceLock::new();

fn with_state<F, R>(f: F) -> Result<R>
where
    F: FnOnce(&mut AppState) -> Result<R>,
{
    let state =
        APP_STATE.get_or_init(|| Mutex::new(AppState::new().expect("Failed to init app state")));
    let mut guard = state.lock().unwrap();
    f(&mut guard)
}

// ─── Request Handler ─────────────────────────────────────────────────────────

pub async fn handle_request(
    req: TuiRequest,
    state: &TuiState,
    event_tx: super::bridge::EventSender,
) -> TuiResponse {
    let id = req.id;
    let cmd = req.cmd.as_str();
    let payload = req.payload.clone().unwrap_or_default();

    let result = match cmd {
        "session.list" => handle_session_list(),
        "session.get" => handle_session_get(&payload),
        "session.create" => handle_session_create(&payload),
        "session.delete" => handle_session_delete(&payload),
        "session.abort" => handle_session_abort(&payload, state).await,
        "session.status" => Ok(json!("idle")),
        "session.messages" => handle_session_messages(&payload),
        "session.diff" => Ok(json!([])),
        "session.todo" => Ok(json!([])),
        "session.fork" => handle_session_get(&payload),
        "session.revert" => Ok(json!({})),
        "session.unrevert" => Ok(json!({})),
        "session.summarize" => Ok(json!({})),
        "session.shell" => handle_session_shell(&payload).await,
        "session.command" => handle_session_command(&payload, state, event_tx).await,
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
        Ok(json!(sessions
            .iter()
            .map(|sess| {
                let title = if sess.task.is_empty() {
                    "New Session"
                } else {
                    &sess.task[..sess.task.len().min(40)]
                };
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
            })
            .collect::<Vec<_>>()))
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
                let title = if session.task.is_empty() {
                    "New Session"
                } else {
                    &session.task[..session.task.len().min(40)]
                };
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
    let task = payload
        .get("task")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    with_state(|s| {
        let mut tracker = s.session_tracker()?;
        let id = tracker.start_session(&task)?;
        *s.current_session.lock().unwrap() = Some(id.clone());
        let title = if task.is_empty() {
            "New Session"
        } else {
            &task[..task.len().min(40)]
        };
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
    if id.is_empty() {
        return Ok(json!({}));
    }
    with_state(|s| {
        let _ = s.session_store()?.delete_session(id);
        if let Some(current) = s.current_session.lock().unwrap().as_ref() {
            if current == id {
                *s.current_session.lock().unwrap() = None;
            }
        }
        Ok(json!({}))
    })
}

fn handle_session_messages(payload: &serde_json::Value) -> Result<serde_json::Value> {
    let id = payload.get("id").and_then(|v| v.as_str()).unwrap_or("");
    if id.is_empty() {
        return Ok(json!([]));
    }
    with_state(|s| match s.session_store()?.load_session(id) {
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
    })
}

async fn handle_session_shell(payload: &serde_json::Value) -> Result<serde_json::Value> {
    let command = payload
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("echo hello");
    let output = tokio::task::block_in_place(|| {
        std::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()
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

async fn handle_session_abort(
    _payload: &serde_json::Value,
    state: &TuiState,
) -> Result<serde_json::Value> {
    if let Some(token) = state.cancel_token.lock().unwrap().take() {
        token.cancel();
    }
    Ok(json!({}))
}

async fn handle_session_command(
    payload: &serde_json::Value,
    state: &TuiState,
    event_tx: super::bridge::EventSender,
) -> Result<serde_json::Value> {
    let session_id = payload
        .get("sessionID")
        .or(payload.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let text = payload
        .get("text")
        .or(payload.get("command"))
        .or(payload.get("task"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if text.is_empty() {
        return Ok(json!({ "id": session_id, "status": "idle" }));
    }

    // Cancel any previously running task
    {
        let mut token_guard = state.cancel_token.lock().unwrap();
        if let Some(token) = token_guard.take() {
            drop(token_guard);
            token.cancel();
        } else {
            drop(token_guard);
        }
    }
    let handle_to_wait = {
        let mut handle_guard = state.task_handle.lock().unwrap();
        handle_guard.take()
    };
    if let Some(handle) = handle_to_wait {
        let _ = handle.await;
    }

    // Clone text for later use
    let text_clone = text.clone();

    let session_id = if session_id.is_empty() || session_id == "new" {
        with_state(|s| {
            let mut tracker = s.session_tracker()?;
            let id = tracker.start_session(&text)?;
            *s.current_session.lock().unwrap() = Some(id.clone());
            Ok::<String, anyhow::Error>(id)
        })?
    } else {
        session_id.to_string()
    };

    let cancel_token = crate::cancellation::CancellationToken::new();
    let cancel_clone = cancel_token.clone();
    *state.cancel_token.lock().unwrap() = Some(cancel_token);

    let event_tx_clone = event_tx.clone();
    let session_id_clone = session_id.clone();

    let handle = tokio::spawn(async move {
        run_task_with_events(session_id_clone, text_clone, event_tx_clone, cancel_clone).await;
    });

    *state.task_handle.lock().unwrap() = Some(handle);

    // Emit session status event
    emit_event(
        &event_tx,
        "session.status",
        &serde_json::json!({
            "sessionID": session_id,
            "status": "busy",
        }),
    );

    // Emit agent started
    emit_event(
        &event_tx,
        "agent.started",
        &serde_json::json!({
            "sessionID": session_id,
            "agent": "main",
            "task": text,
        }),
    );

    Ok(json!({
        "id": session_id,
        "status": "running",
    }))
}

async fn run_task_with_events(
    session_id: String,
    text: String,
    event_tx: super::bridge::EventSender,
    cancel: crate::cancellation::CancellationToken,
) {
    let tx = event_tx.clone();
    let emit = move |event: crate::agent::events::AgentEvent| {
        emit_agent_event(&tx, &event);
    };
    let tx2 = event_tx.clone();
    let session_id2 = session_id.clone();
    let on_chunk = move |chunk: &str| {
        emit_text_delta(&tx2, &session_id2, chunk);
    };

    let result: Result<crate::canonical_runtime::TaskResult, anyhow::Error> = with_state(|s| {
        let mut runtime = s.build_runtime()?;
        let opts = crate::canonical_runtime::TaskOptions {
            cancel: Some(cancel.clone()),
            ..crate::canonical_runtime::TaskOptions::for_mode(
                crate::canonical_runtime::TaskMode::Assist,
            )
        };
        std::thread::spawn(move || {
            let req = crate::canonical_runtime::TaskRequest {
                task: &text,
                conversation: Vec::new(),
                emit: &emit,
                on_chunk: &on_chunk,
            };
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to build tokio runtime");
            rt.block_on(runtime.run_task_with_options(&req, opts))
        })
        .join()
        .map_err(|e| anyhow::anyhow!("Task thread panicked: {:?}", e))
    });

    match result {
        Ok(task_result) => {
            let is_cancelled = task_result.cancelled;
            let success = task_result.success;
            let response = task_result.response;
            let error = task_result.error;

            if is_cancelled {
                emit_agent_event(
                    &event_tx,
                    &crate::agent::events::AgentEvent::AgentCancelled {
                        agent: "main".to_string(),
                    },
                );
                emit_event(
                    &event_tx,
                    "session.status",
                    &serde_json::json!({
                        "sessionID": session_id,
                        "status": "idle",
                    }),
                );
                return;
            }

            if let Some(err) = error {
                emit_agent_event(
                    &event_tx,
                    &crate::agent::events::AgentEvent::AgentFailed {
                        agent: "main".to_string(),
                        error: err.clone(),
                    },
                );
                emit_event(
                    &event_tx,
                    "session.status",
                    &serde_json::json!({
                        "sessionID": session_id,
                        "status": "error",
                    }),
                );
                return;
            }

            // Emit the final assistant message
            let msg_id = format!("assistant-{}", session_id);
            emit_event(
                &event_tx,
                "message.updated",
                &serde_json::json!({
                    "info": {
                        "id": msg_id,
                        "sessionID": session_id,
                        "role": "assistant",
                        "content": response,
                        "parts": [],
                        "time": { "created": chrono::Utc::now().timestamp_millis() },
                    },
                }),
            );

            emit_agent_event(
                &event_tx,
                &crate::agent::events::AgentEvent::AgentCompleted {
                    agent: "main".to_string(),
                    duration_ms: 0,
                },
            );

            emit_event(
                &event_tx,
                "session.status",
                &serde_json::json!({
                    "sessionID": session_id,
                    "status": "idle",
                }),
            );

            // Save to session store
            let _ = with_state(|s| {
                if let Ok(mut tracker) = s.session_tracker() {
                    if let Some(sess) = tracker.current_session_mut() {
                        sess.set_result(&response);
                    }
                    let _ = tracker
                        .store()
                        .save_session(&tracker.current_session().expect("session exists").clone());
                }
                Ok::<(), anyhow::Error>(())
            });
        }
        Err(e) => {
            emit_agent_event(
                &event_tx,
                &crate::agent::events::AgentEvent::AgentFailed {
                    agent: "main".to_string(),
                    error: e.to_string(),
                },
            );
            emit_event(
                &event_tx,
                "session.status",
                &serde_json::json!({
                    "sessionID": session_id,
                    "status": "error",
                }),
            );
        }
    }
}

// ─── Provider Handlers ────────────────────────────────────────────────────────

fn handle_provider_list() -> Result<serde_json::Value> {
    with_state(|s| {
        let pm = s.provider_manager()?;
        Ok(json!(pm
            .list_providers_ordered()
            .iter()
            .map(|(id, entry)| {
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
            })
            .collect::<Vec<_>>()))
    })
}

fn handle_provider_auth(payload: &serde_json::Value) -> Result<serde_json::Value> {
    let provider_id = payload
        .get("providerID")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let api_key = payload.get("apiKey").and_then(|v| v.as_str()).unwrap_or("");
    if provider_id.is_empty() || api_key.is_empty() {
        return Ok(json!({}));
    }
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
    for entry in walkdir::WalkDir::new(cwd)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
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
    let pattern = payload
        .get("pattern")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let cwd = payload.get("cwd").and_then(|v| v.as_str()).unwrap_or(".");
    let mut results = Vec::new();
    for entry in walkdir::WalkDir::new(cwd)
        .max_depth(2)
        .into_iter()
        .filter_map(|e| e.ok())
    {
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
    let command = payload
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("bash");
    let output = tokio::task::block_in_place(|| {
        std::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()
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
    let provider = payload
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let api_key = payload
        .get("auth")
        .and_then(|v| v.get("key"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if provider.is_empty() || api_key.is_empty() {
        return Ok(json!({}));
    }
    with_state(|s| {
        let mut pm = s.provider_manager()?;
        pm.set_api_key(provider, api_key)?;
        let _ = pm.persist();
        Ok(json!({ "status": "ok" }))
    })
}

fn handle_auth_remove(payload: &serde_json::Value) -> Result<serde_json::Value> {
    let provider = payload
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if provider.is_empty() {
        return Ok(json!({}));
    }
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

// ─── Event Helpers ────────────────────────────────────────────────────────────

fn emit_event(tx: &super::bridge::EventSender, event_type: &str, properties: &serde_json::Value) {
    let event = TuiEvent {
        inner: serde_json::json!({
            "id": uuid::Uuid::new_v4().to_string(),
            "type": event_type,
            "properties": properties,
        }),
    };
    let _ = tx.send(event);
}

fn emit_agent_event(tx: &super::bridge::EventSender, event: &crate::agent::events::AgentEvent) {
    let (event_type, properties) = match event {
        crate::agent::events::AgentEvent::AgentStarted { agent, task } => (
            "agent.started",
            serde_json::json!({ "agent": agent, "task": task }),
        ),
        crate::agent::events::AgentEvent::AgentStatusChanged { agent, status } => (
            "agent.status",
            serde_json::json!({ "agent": agent, "status": format!("{:?}", status) }),
        ),
        crate::agent::events::AgentEvent::ToolStarted { tool, args } => (
            "tool.started",
            serde_json::json!({ "tool": tool, "args": args }),
        ),
        crate::agent::events::AgentEvent::ToolCompleted {
            tool,
            result,
            success,
        } => (
            "tool.completed",
            serde_json::json!({ "tool": tool, "result": result, "success": success }),
        ),
        crate::agent::events::AgentEvent::AgentCompleted { agent, duration_ms } => (
            "agent.completed",
            serde_json::json!({ "agent": agent, "duration_ms": duration_ms }),
        ),
        crate::agent::events::AgentEvent::AgentFailed { agent, error } => (
            "agent.failed",
            serde_json::json!({ "agent": agent, "error": error }),
        ),
        crate::agent::events::AgentEvent::AgentCancelled { agent } => {
            ("agent.cancelled", serde_json::json!({ "agent": agent }))
        }
        crate::agent::events::AgentEvent::StreamChunk { content } => {
            return; // Handled separately via emit_text_delta
        }
        crate::agent::events::AgentEvent::Log { level, message } => (
            "log",
            serde_json::json!({ "level": level, "message": message }),
        ),
        _ => return,
    };
    emit_event(tx, event_type, &properties);
}

fn emit_text_delta(tx: &super::bridge::EventSender, session_id: &str, delta: &str) {
    if delta.is_empty() {
        return;
    }
    // Emit as a message.part.delta event for the assistant message
    let part_id = format!("part-{}", uuid::Uuid::new_v4());
    emit_event(
        tx,
        "message.part.delta",
        &serde_json::json!({
            "messageID": format!("assistant-{}", session_id),
            "sessionID": session_id,
            "partID": part_id,
            "field": "value",
            "delta": delta,
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::super::protocol::TuiEvent;
    use super::*;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn test_session_command_empty_text_returns_idle() {
        let state = TuiState::default();
        let (tx, _rx) = mpsc::unbounded_channel::<TuiEvent>();
        let resp = handle_request(
            TuiRequest {
                id: 1,
                cmd: "session.command".to_string(),
                payload: Some(json!({ "text": "" })),
            },
            &state,
            tx,
        )
        .await;
        assert!(resp.result.is_some());
        let data = resp.result.unwrap();
        assert_eq!(data["data"]["status"], "idle");
    }

    #[tokio::test]
    async fn test_session_command_creates_session() {
        let state = TuiState::default();
        let (tx, _rx) = mpsc::unbounded_channel::<TuiEvent>();
        let resp = handle_request(
            TuiRequest {
                id: 2,
                cmd: "session.command".to_string(),
                payload: Some(json!({ "text": "test task" })),
            },
            &state,
            tx,
        )
        .await;
        assert!(resp.result.is_some());
        let data = resp.result.unwrap();
        assert_eq!(data["data"]["status"], "running");
        assert!(!data["data"]["id"].is_null());
    }

    #[tokio::test]
    async fn test_session_list_returns_sessions() {
        let state = TuiState::default();
        let (tx, _rx) = mpsc::unbounded_channel::<TuiEvent>();
        let resp = handle_request(
            TuiRequest {
                id: 3,
                cmd: "session.list".to_string(),
                payload: None,
            },
            &state,
            tx,
        )
        .await;
        assert!(resp.result.is_some());
    }

    #[tokio::test]
    async fn test_unknown_command_returns_error() {
        let state = TuiState::default();
        let (tx, _rx) = mpsc::unbounded_channel::<TuiEvent>();
        let resp = handle_request(
            TuiRequest {
                id: 4,
                cmd: "unknown.cmd".to_string(),
                payload: None,
            },
            &state,
            tx,
        )
        .await;
        assert!(resp.error.is_some());
        assert!(resp.error.as_ref().unwrap().contains("Unknown command"));
    }

    #[test]
    fn test_emit_event_formats_correctly() {
        let (tx, mut rx) = mpsc::unbounded_channel::<TuiEvent>();
        emit_event(&tx, "test.event", &json!({ "key": "value" }));
        drop(tx);

        let event = rx.blocking_recv().expect("event should be received");
        assert_eq!(event.inner["type"], "test.event");
        assert_eq!(event.inner["properties"]["key"], "value");
    }

    #[test]
    fn test_emit_agent_event_maps_agent_started() {
        let (tx, mut rx) = mpsc::unbounded_channel::<TuiEvent>();
        let event = crate::agent::events::AgentEvent::AgentStarted {
            agent: "main".to_string(),
            task: "test task".to_string(),
        };
        emit_agent_event(&tx, &event);
        drop(tx);

        let tui_event = rx.blocking_recv().expect("event should be received");
        assert_eq!(tui_event.inner["type"], "agent.started");
        assert_eq!(tui_event.inner["properties"]["agent"], "main");
        assert_eq!(tui_event.inner["properties"]["task"], "test task");
    }

    #[test]
    fn test_emit_agent_event_maps_tool_started() {
        let (tx, mut rx) = mpsc::unbounded_channel::<TuiEvent>();
        let event = crate::agent::events::AgentEvent::ToolStarted {
            tool: "read_file".to_string(),
            args: "/path/to/file".to_string(),
        };
        emit_agent_event(&tx, &event);
        drop(tx);

        let tui_event = rx.blocking_recv().expect("event should be received");
        assert_eq!(tui_event.inner["type"], "tool.started");
        assert_eq!(tui_event.inner["properties"]["tool"], "read_file");
    }

    #[test]
    fn test_emit_agent_event_maps_tool_completed() {
        let (tx, mut rx) = mpsc::unbounded_channel::<TuiEvent>();
        let event = crate::agent::events::AgentEvent::ToolCompleted {
            tool: "read_file".to_string(),
            result: "file contents".to_string(),
            success: true,
        };
        emit_agent_event(&tx, &event);
        drop(tx);

        let tui_event = rx.blocking_recv().expect("event should be received");
        assert_eq!(tui_event.inner["type"], "tool.completed");
        assert_eq!(tui_event.inner["properties"]["success"], true);
    }

    #[test]
    fn test_emit_agent_event_skips_stream_chunk() {
        let (tx, mut rx) = mpsc::unbounded_channel::<TuiEvent>();
        let event = crate::agent::events::AgentEvent::StreamChunk {
            content: "hello".to_string(),
        };
        emit_agent_event(&tx, &event);
        // StreamChunk is handled separately, so no event should be emitted
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_emit_text_delta_emits_message_part_delta() {
        let (tx, mut rx) = mpsc::unbounded_channel::<TuiEvent>();
        emit_text_delta(&tx, "session-123", "hello world");
        drop(tx);

        let tui_event = rx.blocking_recv().expect("event should be received");
        assert_eq!(tui_event.inner["type"], "message.part.delta");
        assert_eq!(tui_event.inner["properties"]["sessionID"], "session-123");
        assert_eq!(tui_event.inner["properties"]["delta"], "hello world");
    }

    #[test]
    fn test_emit_text_delta_skips_empty_delta() {
        let (tx, mut rx) = mpsc::unbounded_channel::<TuiEvent>();
        emit_text_delta(&tx, "session-123", "");
        drop(tx);
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_handle_session_get_new_session() {
        let resp = handle_session_get(&json!({}));
        assert!(resp.is_ok());
        let data = resp.unwrap();
        assert_eq!(data["id"], "new");
        assert_eq!(data["status"], "idle");
    }

    #[test]
    fn test_handle_session_get_with_id() {
        let resp = handle_session_get(&json!({ "id": "nonexistent" }));
        assert!(resp.is_ok());
    }

    #[test]
    fn test_handle_project_current_returns_directory() {
        let resp = handle_project_current();
        assert!(resp.is_ok());
        let data = resp.unwrap();
        assert!(data["directory"].is_string());
        assert_eq!(data["name"], "CodeBro");
    }

    #[test]
    fn test_handle_config_get_returns_defaults() {
        let resp = handle_config_get();
        assert!(resp.is_ok());
        let data = resp.unwrap();
        assert!(data["provider"].is_string());
        assert_eq!(data["theme"], "dark");
    }
}

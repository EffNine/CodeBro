#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! OpenSandbox HTTP backend.
//!
//! Implements the real OpenSandbox lifecycle API:
//! - `GET /health` — health check
//! - `POST /sandboxes` — create a sandbox container
//! - `GET /sandboxes/{id}/endpoints/44772` — get execd access endpoint
//! - `POST /sandboxes/{id}/proxy/44772/command` — execute command via SSE
//! - `DELETE /sandboxes/{id}` — terminate sandbox
//!
//! The execd (execution daemon) runs inside each sandbox on port 44772.
//! Commands are executed via Server-Sent Events (SSE) and output is parsed
//! from the event stream.
//!
//! Environment variables:
//! - `OPEN_SANDBOX_URL` — base URL of the OpenSandbox server (required)
//! - `OPEN_SANDBOX_API_KEY` — Bearer token for authentication (optional)
//! - `OPEN_SANDBOX_TIMEOUT_SECS` — default sandbox lifetime (default: 120)
//! - `OPEN_SANDBOX_MAX_OUTPUT_BYTES` — max output size (default: 65536)

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use super::{ExecutionResult, SandboxBackend, SandboxCommand, SandboxMode, SandboxPolicy};
use futures::StreamExt;

/// SSE event type from the execd command endpoint.
#[derive(serde::Deserialize, Debug, Clone)]
#[serde(tag = "type")]
enum SseEvent {
    #[serde(rename = "init")]
    Init { text: String, timestamp: i64 },
    #[serde(rename = "ping")]
    Ping { text: String, timestamp: i64 },
    #[serde(rename = "stdout")]
    Stdout { text: String, timestamp: i64 },
    #[serde(rename = "stderr")]
    Stderr { text: String, timestamp: i64 },
    #[serde(rename = "execution_complete")]
    ExecutionComplete { execution_time: u64, timestamp: i64 },
    #[serde(rename = "error")]
    Error { timestamp: i64, error: ExecdError },
}

#[derive(serde::Deserialize, Debug, Clone)]
struct ExecdError {
    #[serde(default)]
    ename: String,
    #[serde(default)]
    evalue: String,
    #[serde(default)]
    traceback: Vec<String>,
}

/// Response from the OpenSandbox lifecycle API.
#[derive(serde::Deserialize, Debug)]
struct CreateSandboxResponse {
    id: String,
    status: SandboxStatus,
    #[serde(alias = "expiresAt")]
    expires_at: Option<String>,
}

#[derive(serde::Deserialize, Debug)]
struct SandboxStatus {
    state: String,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(serde::Deserialize, Debug)]
struct EndpointResponse {
    endpoint: String,
}

/// Request body for creating a sandbox.
#[derive(serde::Serialize, Debug)]
struct CreateSandboxRequest {
    image: ImageSpec,
    entrypoint: Vec<String>,
    #[serde(rename = "resourceLimits")]
    resource_limits: ResourceLimits,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    env: Option<HashMap<String, String>>,
}

#[derive(serde::Serialize, Debug)]
struct ImageSpec {
    uri: String,
}

#[derive(serde::Serialize, Debug)]
struct ResourceLimits {
    #[serde(rename = "cpu")]
    cpu: String,
    #[serde(rename = "memory")]
    memory: String,
}

/// Request body for command execution.
#[derive(serde::Serialize, Debug)]
struct RunCommandRequest {
    command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout: Option<u64>,
    #[serde(rename = "envs", skip_serializing_if = "Option::is_none")]
    envs: Option<HashMap<String, String>>,
}

/// Configuration for the OpenSandbox backend.
#[derive(Debug, Clone)]
pub struct OpenSandboxConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    pub timeout_secs: u64,
    pub max_output_bytes: usize,
    pub image_uri: String,
    pub resource_cpu: String,
    pub resource_memory: String,
}

impl OpenSandboxConfig {
    pub fn from_env() -> Option<Self> {
        let base_url = std::env::var("OPEN_SANDBOX_URL").ok()?;
        let api_key = std::env::var("OPEN_SANDBOX_API_KEY").ok();
        let timeout_secs = std::env::var("OPEN_SANDBOX_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(120);
        let max_output_bytes = std::env::var("OPEN_SANDBOX_MAX_OUTPUT_BYTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(64 * 1024);
        let image_uri = std::env::var("OPEN_SANDBOX_IMAGE")
            .ok()
            .unwrap_or_else(|| "python:3.11-slim".to_string());
        let resource_cpu = std::env::var("OPEN_SANDBOX_RESOURCE_CPU")
            .ok()
            .unwrap_or_else(|| "500m".to_string());
        let resource_memory = std::env::var("OPEN_SANDBOX_RESOURCE_MEMORY")
            .ok()
            .unwrap_or_else(|| "512Mi".to_string());
        Some(OpenSandboxConfig {
            base_url,
            api_key,
            timeout_secs,
            max_output_bytes,
            image_uri,
            resource_cpu,
            resource_memory,
        })
    }
}

/// The OpenSandbox HTTP backend.
///
/// Uses the real lifecycle API: create sandbox → execute command via SSE → delete sandbox.
#[derive(Clone)]
pub struct OpenSandboxBackend {
    config: OpenSandboxConfig,
    client: reqwest::Client,
}

impl OpenSandboxBackend {
    pub fn new(base_url: String) -> Self {
        OpenSandboxBackend {
            config: OpenSandboxConfig {
                base_url: base_url.clone(),
                api_key: std::env::var("OPEN_SANDBOX_API_KEY").ok(),
                timeout_secs: std::env::var("OPEN_SANDBOX_TIMEOUT_SECS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(120),
                max_output_bytes: std::env::var("OPEN_SANDBOX_MAX_OUTPUT_BYTES")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(64 * 1024),
                image_uri: std::env::var("OPEN_SANDBOX_IMAGE")
                    .ok()
                    .unwrap_or_else(|| "python:3.11-slim".to_string()),
                resource_cpu: std::env::var("OPEN_SANDBOX_RESOURCE_CPU")
                    .ok()
                    .unwrap_or_else(|| "500m".to_string()),
                resource_memory: std::env::var("OPEN_SANDBOX_RESOURCE_MEMORY")
                    .ok()
                    .unwrap_or_else(|| "512Mi".to_string()),
            },
            client: reqwest::Client::new(),
        }
    }

    pub fn with_config(config: OpenSandboxConfig) -> Self {
        OpenSandboxBackend {
            config,
            client: reqwest::Client::new(),
        }
    }

    /// Health check: whether the configured endpoint is reachable.
    pub async fn health_check(&self) -> bool {
        let url = format!("{}/health", self.config.base_url);
        match self.client.get(&url).send().await {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }

    /// Execute a command through the OpenSandbox lifecycle API.
    ///
    /// Flow:
    /// 1. Create a sandbox container
    /// 2. Wait for it to reach Running state
    /// 3. Get the execd endpoint (port 44772)
    /// 4. Execute the command via SSE
    /// 5. Parse the SSE stream for stdout/stderr/exit_code
    /// 6. Delete the sandbox
    fn execute(
        &self,
        workspace_root: &PathBuf,
        cmd: SandboxCommand,
        policy: &SandboxPolicy,
    ) -> ExecutionResult {
        let command = cmd.command.trim().to_string();
        let timeout_secs = if policy.timeout_secs > 0 {
            policy.timeout_secs
        } else {
            self.config.timeout_secs
        };
        let ws_root_str = workspace_root.to_string_lossy().to_string();
        let working_dir = cmd
            .working_directory
            .as_deref()
            .unwrap_or(&ws_root_str)
            .to_string();
        let metadata = cmd.metadata.clone();

        let start = Instant::now();

        // Clone everything needed for the async block (run_async requires 'static).
        let backend = self.clone();
        let workspace_root_clone = workspace_root.clone();
        let command_clone = command.clone();
        let working_dir_clone = working_dir.clone();
        let env_clone = policy.env.clone();
        let timeout_secs_clone = timeout_secs;
        let metadata_clone = metadata.clone();
        let metadata_for_error = metadata.clone();

        // Execute the full lifecycle asynchronously.
        let result = run_async(async move {
            backend
                .exec_with_lifecycle(
                    &command_clone,
                    &working_dir_clone,
                    timeout_secs_clone,
                    &env_clone,
                    &workspace_root_clone,
                    start,
                    metadata_clone,
                )
                .await
        });

        match result {
            Ok(r) => r,
            Err(e) => {
                let duration = start.elapsed().as_millis();
                ExecutionResult {
                    command,
                    requested_command: String::new(),
                    resolved_command: String::new(),
                    working_directory: workspace_root.to_string_lossy().to_string(),
                    exit_code: -1,
                    success: false,
                    duration_ms: duration,
                    timestamp: None,
                    stdout: String::new(),
                    stderr: format!("OpenSandbox execution failed: {}", e),
                    timeout: false,
                    cancelled: false,
                    denied: false,
                    denied_reason: Some(format!("execution error: {}", e)),
                    backend: "opensandbox".to_string(),
                    mode: SandboxMode::OpenSandbox.to_string(),
                    execution_id: String::new(),
                    repo_identity: None,
                    repo_state: None,
                    sandbox_capabilities: None,
                    reproducibility: super::Reproducibility::default(),
                    artifacts: Vec::new(),
                    freshness: None,
                    metadata: metadata_for_error,
                }
            }
        }
    }

    async fn exec_with_lifecycle(
        &self,
        command: &str,
        working_dir: &str,
        timeout_secs: u64,
        env: &HashMap<String, String>,
        workspace_root: &PathBuf,
        start: Instant,
        metadata: HashMap<String, String>,
    ) -> Result<ExecutionResult, String> {
        // Step 1: Create sandbox.
        let create_req = CreateSandboxRequest {
            image: ImageSpec {
                uri: self.config.image_uri.clone(),
            },
            entrypoint: vec![
                "tail".to_string(),
                "-f".to_string(),
                "/dev/null".to_string(),
            ],
            resource_limits: ResourceLimits {
                cpu: self.config.resource_cpu.clone(),
                memory: self.config.resource_memory.clone(),
            },
            timeout: Some(std::cmp::max(timeout_secs, 60)), // minimum 60s per API contract
            env: if env.is_empty() {
                None
            } else {
                Some(env.clone())
            },
        };

        let create_url = format!("{}/sandboxes", self.config.base_url);
        let mut req_builder = self.client.post(&create_url).json(&create_req);
        if let Some(ref api_key) = self.config.api_key {
            req_builder = req_builder.header("Authorization", format!("Bearer {}", api_key));
        }

        let resp = req_builder
            .send()
            .await
            .map_err(|e| format!("create sandbox: {}", e))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("create sandbox failed: {} {}", status, body));
        }
        let create_resp: CreateSandboxResponse = resp
            .json()
            .await
            .map_err(|e| format!("parse create response: {}", e))?;
        let sandbox_id = create_resp.id;

        // Step 2: Wait for sandbox to be running.
        self.wait_for_running(&sandbox_id).await?;

        // Step 3: Get execd endpoint.
        let endpoint = self.get_execd_endpoint(&sandbox_id).await?;

        // Step 4: Execute command via SSE.
        let cmd_timeout_ms = if timeout_secs > 0 {
            Some(timeout_secs * 1000)
        } else {
            None
        };
        let (stdout, stderr, exit_code, timeout_flag) = self
            .run_command(&endpoint, command, working_dir, cmd_timeout_ms, env)
            .await?;

        // Step 5: Delete sandbox.
        let _ = self.delete_sandbox(&sandbox_id).await;

        let duration = start.elapsed().as_millis();
        Ok(ExecutionResult {
            command: command.to_string(),
            requested_command: String::new(),
            resolved_command: command.to_string(),
            working_directory: workspace_root.to_string_lossy().to_string(),
            exit_code,
            success: exit_code == 0 && !timeout_flag,
            duration_ms: duration,
            timestamp: None,
            stdout: crate::tools::shell::redact_secrets_public(&stdout),
            stderr: crate::tools::shell::redact_secrets_public(&stderr),
            timeout: timeout_flag,
            cancelled: false,
            denied: false,
            denied_reason: None,
            backend: "opensandbox".to_string(),
            mode: SandboxMode::OpenSandbox.to_string(),
            execution_id: String::new(),
            repo_identity: None,
            repo_state: None,
            sandbox_capabilities: None,
            reproducibility: super::Reproducibility::default(),
            artifacts: Vec::new(),
            freshness: None,
            metadata,
        })
    }

    async fn wait_for_running(&self, sandbox_id: &str) -> Result<(), String> {
        let base_url = self.config.base_url.clone();
        for _ in 0..30 {
            let url = format!("{}/sandboxes/{}", base_url, sandbox_id);
            let mut req_builder = self.client.get(&url);
            if let Some(ref api_key) = self.config.api_key {
                req_builder = req_builder.header("Authorization", format!("Bearer {}", api_key));
            }
            let resp = req_builder
                .send()
                .await
                .map_err(|e| format!("poll sandbox status: {}", e))?;
            if !resp.status().is_success() {
                return Err(format!("poll sandbox status: HTTP {}", resp.status()));
            }
            let info: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            let state = info
                .get("status")
                .and_then(|s| s.get("state"))
                .and_then(|s| s.as_str())
                .unwrap_or("");
            if state == "Running" {
                return Ok(());
            }
            if state == "Failed" || state == "Terminated" {
                return Err(format!("sandbox entered terminal state: {}", state));
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        Err("timed out waiting for sandbox to start".to_string())
    }

    async fn get_execd_endpoint(&self, sandbox_id: &str) -> Result<String, String> {
        let url = format!(
            "{}/sandboxes/{}/endpoints/44772",
            self.config.base_url, sandbox_id
        );
        let mut req_builder = self.client.get(&url);
        if let Some(ref api_key) = self.config.api_key {
            req_builder = req_builder.header("Authorization", format!("Bearer {}", api_key));
        }

        let resp = req_builder
            .send()
            .await
            .map_err(|e| format!("get endpoint: {}", e))?;
        if !resp.status().is_success() {
            return Err(format!("get endpoint: HTTP {}", resp.status()));
        }
        let ep: EndpointResponse = resp.json().await.map_err(|e| e.to_string())?;
        // The endpoint may include a proxy path suffix (e.g. "127.0.0.1:PORT/proxy/44772").
        // Strip it to get the base URL for direct execd access.
        let base = ep
            .endpoint
            .split("/proxy/")
            .next()
            .unwrap_or(&ep.endpoint)
            .to_string();
        Ok(base)
    }

    async fn run_command(
        &self,
        endpoint: &str,
        command: &str,
        cwd: &str,
        timeout_ms: Option<u64>,
        env: &HashMap<String, String>,
    ) -> Result<(String, String, i32, bool), String> {
        let url = format!("http://{}/command", endpoint);
        let req = RunCommandRequest {
            command: command.to_string(),
            cwd: if cwd.is_empty() {
                None
            } else {
                Some(cwd.to_string())
            },
            timeout: timeout_ms,
            envs: if env.is_empty() {
                None
            } else {
                Some(env.clone())
            },
        };

        let mut req_builder = self.client.post(&url).json(&req);
        if let Some(ref api_key) = self.config.api_key {
            req_builder = req_builder.header("Authorization", format!("Bearer {}", api_key));
        }

        let resp = req_builder
            .send()
            .await
            .map_err(|e| format!("run command: {}", e))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("command failed: {} {}", status, body));
        }

        // Parse SSE stream.
        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut exit_code: i32 = 0;
        let mut timeout_flag = false;
        let mut got_complete = false;

        let mut stream = resp.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let bytes = chunk.map_err(|e| format!("read SSE chunk: {}", e))?;
            let text = String::from_utf8_lossy(&bytes);
            for line in text.lines() {
                // Skip empty lines (SSE event separators).
                if line.trim().is_empty() {
                    continue;
                }
                // Process this line directly.
                if OpenSandboxBackend::process_sse_line(
                    line,
                    &mut stdout,
                    &mut stderr,
                    &mut exit_code,
                    &mut timeout_flag,
                    &mut got_complete,
                )? {
                    got_complete = true;
                    break;
                }
            }
            if got_complete || timeout_flag {
                break;
            }
        }

        Ok((stdout, stderr, exit_code, timeout_flag))
    }

    fn process_sse_line(
        line: &str,
        stdout: &mut String,
        stderr: &mut String,
        exit_code: &mut i32,
        timeout_flag: &mut bool,
        got_complete: &mut bool,
    ) -> Result<bool, String> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(false);
        }
        // SSE lines may have "data: " prefix (standard) or be raw JSON.
        let data = trimmed.strip_prefix("data: ").unwrap_or(trimmed).trim();
        if data.is_empty() {
            return Ok(false);
        }
        let event: SseEvent =
            serde_json::from_str(data).map_err(|e| format!("parse SSE: {}", e))?;
        match event {
            SseEvent::Init { .. } | SseEvent::Ping { .. } => {}
            SseEvent::Stdout { text, .. } => stdout.push_str(&text),
            SseEvent::Stderr { text, .. } => stderr.push_str(&text),
            SseEvent::ExecutionComplete { .. } => {
                *got_complete = true;
                *exit_code = 0;
            }
            SseEvent::Error { error, .. } => {
                *got_complete = true;
                if let Ok(code) = error.evalue.parse::<i32>() {
                    *exit_code = code;
                } else if error.ename.contains("timeout") || error.ename.contains("killed") {
                    *timeout_flag = true;
                    *exit_code = -1;
                } else {
                    *exit_code = -1;
                }
            }
        }
        Ok(*got_complete)
    }

    async fn delete_sandbox(&self, sandbox_id: &str) -> Result<(), String> {
        let url = format!("{}/sandboxes/{}", self.config.base_url, sandbox_id);
        let mut req_builder = self.client.delete(&url);
        if let Some(ref api_key) = self.config.api_key {
            req_builder = req_builder.header("Authorization", format!("Bearer {}", api_key));
        }
        let resp = req_builder.send().await.map_err(|e| e.to_string())?;
        if !matches!(
            resp.status(),
            reqwest::StatusCode::NO_CONTENT | reqwest::StatusCode::NOT_FOUND
        ) {
            return Err(format!("delete sandbox: HTTP {}", resp.status()));
        }
        Ok(())
    }
}

impl SandboxBackend for OpenSandboxBackend {
    fn execute(
        &self,
        workspace_root: &PathBuf,
        cmd: SandboxCommand,
        policy: &SandboxPolicy,
    ) -> ExecutionResult {
        self.execute(workspace_root, cmd, policy)
    }

    fn name(&self) -> &str {
        "opensandbox"
    }

    fn mode(&self) -> SandboxMode {
        SandboxMode::OpenSandbox
    }

    fn is_available(&self) -> bool {
        if self.config.base_url.is_empty() {
            return false;
        }
        let url = &self.config.base_url;
        let host = url
            .strip_prefix("http://")
            .unwrap_or(url.strip_prefix("https://").unwrap_or(url));
        let host = host.split('/').next().unwrap_or("");
        let (host, port) = if let Some(colon) = host.rfind(':') {
            (&host[..colon], host[colon + 1..].parse::<u16>().ok())
        } else {
            (host, None)
        };
        let port = port.unwrap_or_else(|| if url.starts_with("https://") { 443 } else { 80 });
        use std::net::TcpStream;
        match TcpStream::connect(format!("{host}:{port}")) {
            Ok(_) => true,
            Err(_) => false,
        }
    }

    fn capabilities(&self) -> super::SandboxCapabilities {
        super::SandboxCapabilities::opensandbox()
    }
}

/// Run an async future to completion in a way that works both inside and
/// outside a tokio test runtime.
fn run_async<T, F: std::future::Future<Output = T> + Send + 'static>(f: F) -> T
where
    T: Send + 'static,
{
    if tokio::runtime::Handle::try_current().is_err() {
        let rt = tokio::runtime::Runtime::new().expect("create tokio runtime");
        return rt.block_on(f);
    }
    let f = Box::pin(f);
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("create tokio runtime in child thread");
        rt.block_on(f)
    })
    .join()
    .expect("child thread panicked")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backend_name() {
        let backend = OpenSandboxBackend::new("http://localhost:9999".to_string());
        assert_eq!(backend.name(), "opensandbox");
        assert_eq!(backend.mode(), SandboxMode::OpenSandbox);
    }

    #[test]
    fn test_backend_unavailable_when_no_url() {
        let backend = OpenSandboxBackend::new(String::new());
        assert!(!backend.is_available());
    }

    #[test]
    fn test_config_from_env_missing() {
        let config = OpenSandboxConfig {
            base_url: "http://test:8080".to_string(),
            api_key: None,
            timeout_secs: 30,
            max_output_bytes: 32768,
            image_uri: "python:3.11-slim".to_string(),
            resource_cpu: "500m".to_string(),
            resource_memory: "512Mi".to_string(),
        };
        let backend = OpenSandboxBackend::with_config(config);
        assert_eq!(backend.config.base_url, "http://test:8080");
        assert_eq!(backend.config.timeout_secs, 30);
        assert_eq!(backend.config.image_uri, "python:3.11-slim");
    }

    #[test]
    fn test_create_request_serializes_correctly() {
        let req = CreateSandboxRequest {
            image: ImageSpec {
                uri: "python:3.11-slim".to_string(),
            },
            entrypoint: vec![
                "tail".to_string(),
                "-f".to_string(),
                "/dev/null".to_string(),
            ],
            resource_limits: ResourceLimits {
                cpu: "500m".to_string(),
                memory: "512Mi".to_string(),
            },
            timeout: Some(120),
            env: None,
        };
        let json = serde_json::to_string(&req).expect("serialize");
        let v: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert_eq!(v["image"]["uri"], "python:3.11-slim");
        assert_eq!(
            v["entrypoint"],
            serde_json::json!(["tail", "-f", "/dev/null"])
        );
        assert_eq!(v["resourceLimits"]["cpu"], "500m");
        assert_eq!(v["timeout"], 120);
    }

    #[test]
    fn test_run_command_request_serializes_correctly() {
        let req = RunCommandRequest {
            command: "echo hello".to_string(),
            cwd: Some("/workspace".to_string()),
            timeout: Some(10000),
            envs: None,
        };
        let json = serde_json::to_string(&req).expect("serialize");
        let v: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert_eq!(v["command"], "echo hello");
        assert_eq!(v["cwd"], "/workspace");
        assert_eq!(v["timeout"], 10000);
        assert!(!v.get("envs").is_some());
    }

    #[test]
    fn test_sse_event_parsing() {
        let init = r#"{"type":"init","text":"abc123","timestamp":1000}"#;
        let ev: SseEvent = serde_json::from_str(init).expect("parse init");
        match ev {
            SseEvent::Init { text, .. } => assert_eq!(text, "abc123"),
            _ => panic!("expected Init"),
        }

        let stdout = r#"{"type":"stdout","text":"hello\n","timestamp":1001}"#;
        let ev: SseEvent = serde_json::from_str(stdout).expect("parse stdout");
        match ev {
            SseEvent::Stdout { text, .. } => assert_eq!(text, "hello\n"),
            _ => panic!("expected Stdout"),
        }

        let stderr = r#"{"type":"stderr","text":"error msg\n","timestamp":1002}"#;
        let ev: SseEvent = serde_json::from_str(stderr).expect("parse stderr");
        match ev {
            SseEvent::Stderr { text, .. } => assert_eq!(text, "error msg\n"),
            _ => panic!("expected Stderr"),
        }

        let complete = r#"{"type":"execution_complete","execution_time":50,"timestamp":1050}"#;
        let ev: SseEvent = serde_json::from_str(complete).expect("parse complete");
        match ev {
            SseEvent::ExecutionComplete { execution_time, .. } => assert_eq!(execution_time, 50),
            _ => panic!("expected ExecutionComplete"),
        }

        let error = r#"{"type":"error","timestamp":1051,"error":{"ename":"CommandExecError","evalue":"1","traceback":["exit status 1"]}}"#;
        let ev: SseEvent = serde_json::from_str(error).expect("parse error");
        match ev {
            SseEvent::Error { error: e, .. } => {
                assert_eq!(e.ename, "CommandExecError");
                assert_eq!(e.evalue, "1");
            }
            _ => panic!("expected Error"),
        }
    }

    #[test]
    fn test_exit_code_from_sse_events() {
        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut exit_code = 0;
        let mut timeout_flag = false;
        let mut got_complete = false;

        // Simulate successful command.
        let lines = vec![
            r#"data: {"type":"init","text":"exec-1","timestamp":1}"#,
            r#"data: {"type":"ping","text":"pong","timestamp":1}"#,
            r#"data: {"type":"stdout","text":"hello\n","timestamp":2}"#,
            r#"data: {"type":"execution_complete","execution_time":10,"timestamp":3}"#,
        ];
        for line in &lines {
            let done = OpenSandboxBackend::process_sse_line(
                line,
                &mut stdout,
                &mut stderr,
                &mut exit_code,
                &mut timeout_flag,
                &mut got_complete,
            )
            .unwrap();
            if done {
                break;
            }
        }
        assert_eq!(stdout, "hello\n");
        assert_eq!(exit_code, 0);
        assert!(!timeout_flag);

        // Simulate failing command.
        let mut stdout2 = String::new();
        let mut stderr2 = String::new();
        let mut exit_code2 = 0;
        let mut timeout_flag2 = false;
        let mut got_complete2 = false;
        let lines2 = vec![
            r#"data: {"type":"init","text":"exec-2","timestamp":1}"#,
            r#"data: {"type":"stdout","text":"out\n","timestamp":2}"#,
            r#"data: {"type":"stderr","text":"err\n","timestamp":3}"#,
            r#"data: {"type":"error","timestamp":4,"error":{"ename":"CommandExecError","evalue":"1","traceback":["exit status 1"]}}"#,
        ];
        for line in &lines2 {
            let done = OpenSandboxBackend::process_sse_line(
                line,
                &mut stdout2,
                &mut stderr2,
                &mut exit_code2,
                &mut timeout_flag2,
                &mut got_complete2,
            )
            .unwrap();
            if done {
                break;
            }
        }
        assert_eq!(stdout2, "out\n");
        assert_eq!(stderr2, "err\n");
        assert_eq!(exit_code2, 1);
        assert!(!timeout_flag2);
    }

    /// Live OpenSandbox integration test.
    ///
    /// Requires `OPEN_SANDBOX_URL` to be set and `OPEN_SANDBOX_INTEGRATION=1`
    /// to be enabled. Skips cleanly when the environment is not configured.
    #[tokio::test]
    async fn test_opensandbox_live_integration() {
        let url = match std::env::var("OPEN_SANDBOX_URL") {
            Ok(u) if !u.is_empty() => u,
            _ => {
                eprintln!("SKIP: OPEN_SANDBOX_URL not set");
                return;
            }
        };
        if std::env::var("OPEN_SANDBOX_INTEGRATION").unwrap_or_default() != "1" {
            eprintln!("SKIP: OPEN_SANDBOX_INTEGRATION != 1");
            return;
        }
        let backend = OpenSandboxBackend::new(url);
        if !backend.health_check().await {
            eprintln!("SKIP: OpenSandbox service unreachable");
            return;
        }
        let workspace = std::path::PathBuf::from("/tmp");
        let cmd = SandboxCommand {
            command: "echo live-integration-ok".to_string(),
            working_directory: None,
            policy: None,
            metadata: HashMap::new(),
        };
        let policy = SandboxPolicy::new().with_timeout(10);
        let result = backend.execute(&workspace, cmd, &policy);
        assert_eq!(result.backend, "opensandbox");
        assert_eq!(result.mode, "opensandbox");
        assert!(result.success);
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("live-integration-ok"));
    }

    /// Live test: successful command execution.
    #[tokio::test]
    async fn test_opensandbox_exec_success() {
        if std::env::var("OPEN_SANDBOX_INTEGRATION").unwrap_or_default() != "1" {
            eprintln!("SKIP: OPEN_SANDBOX_INTEGRATION != 1");
            return;
        }
        let url = std::env::var("OPEN_SANDBOX_URL").expect("OPEN_SANDBOX_URL set");
        let backend = OpenSandboxBackend::new(url);
        let workspace = std::path::PathBuf::from("/tmp");
        let cmd = SandboxCommand {
            command: "echo hello-world".to_string(),
            working_directory: None,
            policy: None,
            metadata: HashMap::new(),
        };
        let policy = SandboxPolicy::new().with_timeout(10);
        let result = backend.execute(&workspace, cmd, &policy);
        assert_eq!(result.backend, "opensandbox");
        assert!(result.success);
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("hello-world"));
    }

    /// Live test: failing command returns non-zero exit code.
    #[tokio::test]
    async fn test_opensandbox_exec_failure() {
        if std::env::var("OPEN_SANDBOX_INTEGRATION").unwrap_or_default() != "1" {
            eprintln!("SKIP: OPEN_SANDBOX_INTEGRATION != 1");
            return;
        }
        let url = std::env::var("OPEN_SANDBOX_URL").expect("OPEN_SANDBOX_URL set");
        let backend = OpenSandboxBackend::new(url);
        let workspace = std::path::PathBuf::from("/tmp");
        let cmd = SandboxCommand {
            command: "false".to_string(),
            working_directory: None,
            policy: None,
            metadata: HashMap::new(),
        };
        let policy = SandboxPolicy::new().with_timeout(10);
        let result = backend.execute(&workspace, cmd, &policy);
        assert_eq!(result.backend, "opensandbox");
        assert!(!result.success);
        assert_eq!(result.exit_code, 1);
    }

    /// Live test: timeout semantics.
    #[tokio::test]
    async fn test_opensandbox_exec_timeout() {
        if std::env::var("OPEN_SANDBOX_INTEGRATION").unwrap_or_default() != "1" {
            eprintln!("SKIP: OPEN_SANDBOX_INTEGRATION != 1");
            return;
        }
        let url = std::env::var("OPEN_SANDBOX_URL").expect("OPEN_SANDBOX_URL set");
        let backend = OpenSandboxBackend::new(url);
        let workspace = std::path::PathBuf::from("/tmp");
        let cmd = SandboxCommand {
            command: "sleep 30".to_string(),
            working_directory: None,
            policy: None,
            metadata: HashMap::new(),
        };
        let policy = SandboxPolicy::new().with_timeout(3);
        let result = backend.execute(&workspace, cmd, &policy);
        assert_eq!(result.backend, "opensandbox");
        assert!(!result.success);
        // The command should have been terminated.
        assert!(result.timeout || result.exit_code != 0);
    }

    /// Live test: stdout and stderr separation.
    #[tokio::test]
    async fn test_opensandbox_exec_stdout_stderr() {
        if std::env::var("OPEN_SANDBOX_INTEGRATION").unwrap_or_default() != "1" {
            eprintln!("SKIP: OPEN_SANDBOX_INTEGRATION != 1");
            return;
        }
        let url = std::env::var("OPEN_SANDBOX_URL").expect("OPEN_SANDBOX_URL set");
        let backend = OpenSandboxBackend::new(url);
        let workspace = std::path::PathBuf::from("/tmp");
        let cmd = SandboxCommand {
            command: "echo stdout-msg; echo stderr-msg >&2".to_string(),
            working_directory: None,
            policy: None,
            metadata: HashMap::new(),
        };
        let policy = SandboxPolicy::new().with_timeout(10);
        let result = backend.execute(&workspace, cmd, &policy);
        assert_eq!(result.backend, "opensandbox");
        assert!(result.success);
        assert!(result.stdout.contains("stdout-msg"));
        assert!(result.stderr.contains("stderr-msg"));
    }

    /// Integration: unreachable OpenSandbox URL returns structured error.
    #[tokio::test]
    async fn test_opensandbox_unreachable_returns_structured_error() {
        let backend = OpenSandboxBackend::new("http://localhost:1".to_string());
        let workspace = std::path::PathBuf::from("/tmp");
        let cmd = SandboxCommand {
            command: "echo hi".to_string(),
            working_directory: None,
            policy: None,
            metadata: HashMap::new(),
        };
        let policy = SandboxPolicy::new().with_timeout(5);
        let result = backend.execute(&workspace, cmd, &policy);
        assert_eq!(result.backend, "opensandbox");
        assert_eq!(result.exit_code, -1);
        assert!(!result.success);
        assert!(!result.stderr.is_empty());
    }
}

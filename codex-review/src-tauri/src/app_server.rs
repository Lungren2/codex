use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use thiserror::Error;

use crate::models::{ValidationResult, ValidationStatus};

const PROTOCOL_VERSION: &str = "2.0";
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug)]
pub struct AppServerConfig {
    pub codex_executable: PathBuf,
    pub request_timeout: Duration,
    pub turn_timeout: Duration,
}

impl Default for AppServerConfig {
    fn default() -> Self {
        Self {
            codex_executable: PathBuf::from("codex"),
            request_timeout: Duration::from_secs(15),
            turn_timeout: Duration::from_secs(180),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SynthesisTurnRequest {
    pub cwd: PathBuf,
    pub prompt: String,
    pub output_schema: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SynthesisTurnResponse {
    pub thread_id: String,
    pub turn_id: String,
    pub message: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnEvidence {
    pub status: String,
    pub original_request: String,
    pub final_response: String,
    pub validations: Vec<ValidationResult>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReconciledCompletion {
    pub thread_id: String,
    pub turn_id: String,
    pub cwd: PathBuf,
    pub input_messages: Vec<String>,
    pub last_assistant_message: Option<String>,
    pub completed_at: i64,
}

#[derive(Debug, Error)]
pub enum AppServerError {
    #[error("failed to start Codex app-server: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("app-server transport failed: {0}")]
    Transport(String),
    #[error("app-server request `{method}` failed: {message}")]
    Rpc { method: String, message: String },
    #[error("app-server request `{0}` timed out")]
    Timeout(String),
    #[error("app-server returned an invalid `{0}` response")]
    InvalidResponse(&'static str),
    #[error("synthesis turn ended with status `{status}`: {message}")]
    TurnFailed { status: String, message: String },
    #[error("synthesis produced no agent message")]
    MissingAgentMessage,
    #[error("synthesis agent message was not valid JSON: {0}")]
    InvalidAgentMessage(#[source] serde_json::Error),
}

pub struct AppServerClient {
    child: Child,
    stdin: ChildStdin,
    messages: Receiver<Result<Value, String>>,
    pending: HashMap<u64, Value>,
    pending_notifications: VecDeque<Value>,
    next_id: u64,
    request_timeout: Duration,
    turn_timeout: Duration,
}

impl AppServerClient {
    pub fn spawn(config: &AppServerConfig) -> Result<Self, AppServerError> {
        let mut child = Command::new(&config.codex_executable)
            .args(["app-server", "--stdio", "--disable", "hooks"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(AppServerError::Spawn)?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AppServerError::Transport("app-server stdin was unavailable".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AppServerError::Transport("app-server stdout was unavailable".into()))?;
        let (sender, messages) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let parsed = line.map_err(|error| error.to_string()).and_then(|line| {
                    serde_json::from_str(&line).map_err(|error| error.to_string())
                });
                if sender.send(parsed).is_err() {
                    break;
                }
            }
        });

        let mut client = Self {
            child,
            stdin,
            messages,
            pending: HashMap::new(),
            pending_notifications: VecDeque::new(),
            next_id: 1,
            request_timeout: config.request_timeout,
            turn_timeout: config.turn_timeout,
        };
        client.initialize()?;
        Ok(client)
    }

    pub fn run_synthesis_turn(
        &mut self,
        request: &SynthesisTurnRequest,
    ) -> Result<SynthesisTurnResponse, AppServerError> {
        let cwd = absolute_utf8_path(&request.cwd)?;
        let thread = self.request(
            "thread/start",
            json!({
                "cwd": cwd,
                "approvalPolicy": "never",
                "sandbox": "read-only",
                "ephemeral": true,
                "serviceName": "codex-review-synthesis"
            }),
            self.request_timeout,
        )?;
        let thread_id = string_at(&thread, &["thread", "id"], "thread/start")?;
        let turn = self.request(
            "turn/start",
            json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": request.prompt }],
                "outputSchema": request.output_schema
            }),
            self.request_timeout,
        )?;
        let turn_id = string_at(&turn, &["turn", "id"], "turn/start")?;

        let result = self.await_turn(&thread_id, &turn_id);
        if matches!(result, Err(AppServerError::Timeout(_))) {
            let _ = self.request(
                "turn/interrupt",
                json!({ "threadId": thread_id, "turnId": turn_id }),
                CLEANUP_TIMEOUT,
            );
        }
        let _ = self.request(
            "thread/unsubscribe",
            json!({ "threadId": thread_id }),
            CLEANUP_TIMEOUT,
        );
        result.map(|message| SynthesisTurnResponse {
            thread_id,
            turn_id,
            message,
        })
    }

    pub fn read_turn_evidence(
        &mut self,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<TurnEvidence, AppServerError> {
        let response = self.request(
            "thread/read",
            json!({ "threadId": thread_id, "includeTurns": true }),
            self.request_timeout,
        )?;
        extract_turn_evidence(&response, turn_id)
    }

    pub fn list_recent_completions(
        &mut self,
        completed_since: i64,
    ) -> Result<Vec<ReconciledCompletion>, AppServerError> {
        let response = self.request(
            "thread/list",
            json!({
                "limit": 30,
                "sourceKinds": ["vscode"],
                "archived": false,
                "sortKey": "updated_at",
                "sortDirection": "desc",
                "useStateDbOnly": true
            }),
            self.request_timeout,
        )?;
        let threads = response
            .get("data")
            .and_then(Value::as_array)
            .ok_or(AppServerError::InvalidResponse("thread/list data"))?;
        let identities = threads
            .iter()
            .filter_map(|thread| {
                Some((
                    thread.get("id")?.as_str()?.to_string(),
                    PathBuf::from(thread.get("cwd")?.as_str()?),
                ))
            })
            .collect::<Vec<_>>();
        let mut completions = Vec::new();
        for (thread_id, cwd) in identities {
            let read = self.request(
                "thread/read",
                json!({ "threadId": thread_id, "includeTurns": true }),
                self.request_timeout,
            )?;
            for turn in read
                .pointer("/thread/turns")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let completed_at = turn
                    .get("completedAt")
                    .and_then(Value::as_i64)
                    .unwrap_or_default();
                if completed_at < completed_since
                    || turn.get("status").and_then(Value::as_str) != Some("completed")
                {
                    continue;
                }
                let Some(turn_id) = turn.get("id").and_then(Value::as_str) else {
                    continue;
                };
                let evidence = extract_turn_evidence(&read, turn_id)?;
                completions.push(ReconciledCompletion {
                    thread_id: thread_id.clone(),
                    turn_id: turn_id.to_string(),
                    cwd: cwd.clone(),
                    input_messages: evidence
                        .original_request
                        .split("\n\n")
                        .filter(|value| !value.is_empty())
                        .map(str::to_string)
                        .collect(),
                    last_assistant_message: (!evidence.final_response.is_empty())
                        .then_some(evidence.final_response),
                    completed_at,
                });
            }
        }
        Ok(completions)
    }

    fn initialize(&mut self) -> Result<(), AppServerError> {
        self.request(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "codex_review_companion",
                    "title": "Codex Review Companion",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": {
                    "experimentalApi": true,
                    "requestAttestation": false,
                    "mcpServerOpenaiFormElicitation": false,
                    "optOutNotificationMethods": ["item/agentMessage/delta"]
                }
            }),
            self.request_timeout,
        )?;
        self.write(&json!({ "jsonrpc": PROTOCOL_VERSION, "method": "initialized" }))
    }

    fn await_turn(&mut self, thread_id: &str, turn_id: &str) -> Result<Value, AppServerError> {
        let deadline = Instant::now() + self.turn_timeout;
        let mut final_message = None;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(AppServerError::Timeout("turn/completed".into()));
            }
            let message = self.next_notification(remaining)?;
            if is_server_request(&message) {
                self.reject_server_request(&message)?;
                continue;
            }
            match message.get("method").and_then(Value::as_str) {
                Some("item/completed") => {
                    let matching_thread = message
                        .pointer("/params/threadId")
                        .and_then(Value::as_str)
                        .is_none_or(|id| id == thread_id);
                    let matching_turn = message
                        .pointer("/params/turnId")
                        .and_then(Value::as_str)
                        .is_none_or(|id| id == turn_id);
                    if matching_thread
                        && matching_turn
                        && let Some(item) = message.pointer("/params/item").filter(|item| {
                            item.get("type").and_then(Value::as_str) == Some("agentMessage")
                                && item.get("text").and_then(Value::as_str).is_some()
                        })
                    {
                        final_message = item.get("text").and_then(Value::as_str).map(str::to_owned);
                    }
                }
                Some("turn/completed") => {
                    let completed_turn_id =
                        message.pointer("/params/turn/id").and_then(Value::as_str);
                    let completed_thread_id = message
                        .pointer("/params/threadId")
                        .and_then(Value::as_str)
                        .or_else(|| message.pointer("/params/thread/id").and_then(Value::as_str));
                    if completed_turn_id != Some(turn_id)
                        || completed_thread_id.is_some_and(|id| id != thread_id)
                    {
                        continue;
                    }
                    let status = message
                        .pointer("/params/turn/status")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    if status != "completed" {
                        let detail = message
                            .pointer("/params/turn/error/message")
                            .and_then(Value::as_str)
                            .unwrap_or("no error detail was provided")
                            .to_owned();
                        return Err(AppServerError::TurnFailed {
                            status: status.to_owned(),
                            message: detail,
                        });
                    }
                    let text = final_message.ok_or(AppServerError::MissingAgentMessage)?;
                    return serde_json::from_str(&text)
                        .map_err(AppServerError::InvalidAgentMessage);
                }
                _ => {}
            }
        }
    }

    fn request(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, AppServerError> {
        let id = self.next_id;
        self.next_id += 1;
        self.write(&json!({
            "jsonrpc": PROTOCOL_VERSION,
            "id": id,
            "method": method,
            "params": params
        }))?;
        if let Some(response) = self.pending.remove(&id) {
            return rpc_result(method, response);
        }
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(AppServerError::Timeout(method.to_owned()));
            }
            let message = self.receive(remaining)?;
            if is_server_request(&message) {
                self.reject_server_request(&message)?;
            } else if let Some(response_id) = message.get("id").and_then(Value::as_u64) {
                if response_id == id {
                    return rpc_result(method, message);
                }
                self.pending.insert(response_id, message);
            } else if message.get("method").is_some() {
                self.pending_notifications.push_back(message);
            }
        }
    }

    fn next_notification(&mut self, timeout: Duration) -> Result<Value, AppServerError> {
        self.pending_notifications
            .pop_front()
            .map(Ok)
            .unwrap_or_else(|| self.receive(timeout))
    }

    fn receive(&self, timeout: Duration) -> Result<Value, AppServerError> {
        match self.messages.recv_timeout(timeout) {
            Ok(Ok(message)) => Ok(message),
            Ok(Err(error)) => Err(AppServerError::Transport(error)),
            Err(RecvTimeoutError::Timeout) => Err(AppServerError::Timeout("response".into())),
            Err(RecvTimeoutError::Disconnected) => {
                Err(AppServerError::Transport("app-server stdout closed".into()))
            }
        }
    }

    fn reject_server_request(&mut self, message: &Value) -> Result<(), AppServerError> {
        self.write(&json!({
            "jsonrpc": PROTOCOL_VERSION,
            "id": message.get("id").cloned().unwrap_or(Value::Null),
            "error": {
                "code": -32001,
                "message": "Codex Review synthesis is read-only and rejects interactive server requests"
            }
        }))
    }

    fn write(&mut self, value: &Value) -> Result<(), AppServerError> {
        serde_json::to_writer(&mut self.stdin, value)
            .map_err(|error| AppServerError::Transport(error.to_string()))?;
        self.stdin
            .write_all(b"\n")
            .and_then(|_| self.stdin.flush())
            .map_err(|error| AppServerError::Transport(error.to_string()))
    }
}

pub fn read_desktop_turn(
    config: &AppServerConfig,
    thread_id: &str,
    turn_id: &str,
) -> Result<TurnEvidence, AppServerError> {
    AppServerClient::spawn(config)?.read_turn_evidence(thread_id, turn_id)
}

pub fn recent_desktop_completions(
    config: &AppServerConfig,
    completed_since: i64,
) -> Result<Vec<ReconciledCompletion>, AppServerError> {
    AppServerClient::spawn(config)?.list_recent_completions(completed_since)
}

impl Drop for AppServerClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn absolute_utf8_path(path: &Path) -> Result<String, AppServerError> {
    if !path.is_absolute() {
        return Err(AppServerError::Transport(
            "synthesis workspace path must be absolute".into(),
        ));
    }
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| AppServerError::Transport("workspace path is not valid UTF-8".into()))
}

fn is_server_request(message: &Value) -> bool {
    message.get("method").is_some() && message.get("id").is_some()
}

fn rpc_result(method: &str, response: Value) -> Result<Value, AppServerError> {
    if let Some(error) = response.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown JSON-RPC error")
            .to_owned();
        return Err(AppServerError::Rpc {
            method: method.to_owned(),
            message,
        });
    }
    response
        .get("result")
        .cloned()
        .ok_or(AppServerError::InvalidResponse("JSON-RPC"))
}

fn string_at(
    value: &Value,
    path: &[&str],
    response: &'static str,
) -> Result<String, AppServerError> {
    let mut current = value;
    for segment in path {
        current = current
            .get(segment)
            .ok_or(AppServerError::InvalidResponse(response))?;
    }
    current
        .as_str()
        .map(str::to_owned)
        .ok_or(AppServerError::InvalidResponse(response))
}

fn extract_turn_evidence(response: &Value, turn_id: &str) -> Result<TurnEvidence, AppServerError> {
    let turns = response
        .pointer("/thread/turns")
        .and_then(Value::as_array)
        .ok_or(AppServerError::InvalidResponse("thread/read turns"))?;
    let turn = turns
        .iter()
        .find(|turn| turn.get("id").and_then(Value::as_str) == Some(turn_id))
        .ok_or(AppServerError::InvalidResponse("thread/read turn id"))?;
    let status = turn
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let mut requests = Vec::new();
    let mut final_response = String::new();
    let mut validations = Vec::new();
    for item in turn
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match item.get("type").and_then(Value::as_str) {
            Some("userMessage") => {
                for content in item
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if content.get("type").and_then(Value::as_str) == Some("text")
                        && let Some(text) = content.get("text").and_then(Value::as_str)
                    {
                        requests.push(text.to_string());
                    }
                }
            }
            Some("agentMessage") => {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    final_response = text.to_string();
                }
            }
            Some("commandExecution") => {
                let command = item
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or("command");
                let exit_code = item.get("exitCode").and_then(Value::as_i64);
                let command_status = item
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let status = if command_status == "completed" && exit_code == Some(0) {
                    ValidationStatus::Passed
                } else if matches!(command_status, "failed" | "declined")
                    || exit_code.is_some_and(|code| code != 0)
                {
                    ValidationStatus::Failed
                } else {
                    ValidationStatus::NotRun
                };
                let evidence = item
                    .get("aggregatedOutput")
                    .and_then(Value::as_str)
                    .map(|output| truncate_chars(output, 2_000));
                validations.push(ValidationResult {
                    id: format!("command-{}", validations.len() + 1),
                    label: truncate_chars(command, 240),
                    status,
                    evidence,
                });
            }
            _ => {}
        }
    }
    Ok(TurnEvidence {
        status,
        original_request: requests.join("\n\n"),
        final_response,
        validations,
    })
}

fn truncate_chars(value: &str, cap: usize) -> String {
    value.chars().take(cap).collect()
}

#[cfg(test)]
#[path = "app_server_tests.rs"]
mod tests;

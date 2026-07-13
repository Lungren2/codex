use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CompletionPayload {
    pub event_type: String,
    pub thread_id: String,
    pub turn_id: String,
    pub cwd: PathBuf,
    pub status: Option<String>,
    pub origin: Option<String>,
    pub input_messages: Vec<String>,
    pub last_assistant_message: Option<String>,
    pub raw: Value,
}

impl CompletionPayload {
    pub fn parse(json: &str) -> Result<Self> {
        let raw: Value = serde_json::from_str(json).context("notify payload is not valid JSON")?;
        let object = raw
            .as_object()
            .context("notify payload must be a JSON object")?;
        let event_type = string(object, &["type", "eventType", "event_type"])
            .context("notify payload has no event type")?;
        if event_type != "agent-turn-complete" {
            bail!("unsupported notify event `{event_type}`");
        }
        let thread_id = string(
            object,
            &["threadId", "thread-id", "thread_id", "conversationId"],
        )
        .context("completion payload has no thread id")?;
        let turn_id = string(object, &["turnId", "turn-id", "turn_id"])
            .context("completion payload has no turn id")?;
        let cwd = string(object, &["cwd", "workingDirectory", "working-directory"])
            .map(PathBuf::from)
            .context("completion payload has no working directory")?;
        let input_messages = object
            .get("input-messages")
            .or_else(|| object.get("inputMessages"))
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            event_type,
            thread_id,
            turn_id,
            cwd,
            status: string(object, &["status"]),
            origin: string(object, &["origin", "client"]),
            input_messages,
            last_assistant_message: string(
                object,
                &["lastAssistantMessage", "last-assistant-message"],
            ),
            raw,
        })
    }

    pub fn should_capture(&self) -> bool {
        let successful = self
            .status
            .as_deref()
            .is_none_or(|status| matches!(status, "completed" | "success" | "succeeded"));
        let desktop = self
            .origin
            .as_deref()
            .is_none_or(|origin| matches!(origin, "desktop" | "codex-desktop"));
        successful && desktop
    }
}

fn string(object: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str).map(str::to_owned))
}

#[cfg(test)]
#[path = "notify_tests.rs"]
mod tests;

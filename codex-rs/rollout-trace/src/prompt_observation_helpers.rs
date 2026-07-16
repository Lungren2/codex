use serde_json::Value;
use sha1::Digest;
use sha1::Sha1;

use super::PromptInventoryEntry;

pub(super) fn classify_subsystems(item_type: &str, role: Option<&str>, text: &str) -> Vec<String> {
    if item_type == "additional_tools" {
        return vec![
            "client::responses_lite".to_string(),
            "core::tools::spec_plan".to_string(),
        ];
    }
    let markers = [
        ("<permissions instructions>", "prompts::permissions"),
        ("<collaboration_mode>", "collaboration_mode_templates"),
        ("<skills_instructions>", "core::skills"),
        ("<apps_instructions>", "core::apps"),
        ("<plugins_instructions>", "core::plugins"),
        ("<recommended_plugins>", "core::recommended_plugins"),
        ("# AGENTS.md instructions", "core::user_instructions"),
        ("<environment_context>", "core::environment_context"),
        ("<app-context>", "host::app_context"),
        ("<multi_agent_mode>", "core::multi_agent_mode"),
    ];
    let mut subsystems = markers
        .iter()
        .filter(|(marker, _)| text.contains(marker))
        .map(|(_, subsystem)| (*subsystem).to_string())
        .collect::<Vec<_>>();
    if subsystems.is_empty() {
        subsystems.push(match role {
            Some("user") => "conversation::user".to_string(),
            Some("assistant") => "conversation::assistant".to_string(),
            Some("developer") => "core::developer_context".to_string(),
            _ => "conversation::history".to_string(),
        });
    }
    subsystems
}

pub(super) fn looks_contextual(text: &str) -> bool {
    text.contains("<environment_context>")
        || text.contains("# AGENTS.md instructions")
        || text.contains("<recommended_plugins>")
        || text.contains("<multi_agent_mode>")
}

pub(super) fn additional_tools(input: &[Value]) -> Vec<Value> {
    input
        .iter()
        .find(|item| item.get("type").and_then(Value::as_str) == Some("additional_tools"))
        .and_then(|item| item.get("tools"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

pub(super) fn collect_text(value: &Value) -> String {
    if let Value::String(text) = value {
        return text.clone();
    }
    let mut text = Vec::new();
    collect_text_into(value, &mut text);
    text.join("\n")
}

fn collect_text_into(value: &Value, output: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if matches!(key.as_str(), "text" | "description" | "instructions")
                    && let Some(value) = value.as_str()
                {
                    output.push(value.to_string());
                } else {
                    collect_text_into(value, output);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_text_into(value, output);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

pub(super) fn count_image_details(value: &Value) -> usize {
    match value {
        Value::Object(map) => {
            usize::from(
                map.get("type").and_then(Value::as_str) == Some("input_image")
                    && map.get("detail").is_some_and(|detail| !detail.is_null()),
            ) + map.values().map(count_image_details).sum::<usize>()
        }
        Value::Array(values) => values.iter().map(count_image_details).sum(),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => 0,
    }
}

pub(super) fn canonicalize(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries = map.into_iter().collect::<Vec<_>>();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, canonicalize(value)))
                    .collect(),
            )
        }
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize).collect()),
        value => value,
    }
}

pub(super) fn stable_hash(value: &Value) -> String {
    let bytes = serde_json::to_vec(&canonicalize(value.clone())).unwrap_or_default();
    format!("{:x}", Sha1::digest(bytes))
}

pub(super) fn format_entry(label: &str, entry: &PromptInventoryEntry) -> String {
    format!(
        "{label}: role={} type={} provenance={} producer={} chars={} serialized_chars={} estimated_tokens={} sha1={}",
        entry.role.as_deref().unwrap_or("-"),
        entry.item_type,
        entry.provenance,
        entry.producing_subsystems.join(","),
        entry.character_count,
        entry.serialized_character_count,
        entry.estimated_tokens,
        entry.sha1,
    )
}

//! Diagnostic projection of the exact provider request and its pre-transform prompt assembly.

use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use codex_protocol::models::ResponseItem;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

use crate::InferenceCall;
use crate::RolloutTrace;
use crate::replay_bundle;

#[path = "prompt_observation_helpers.rs"]
mod helpers;
use helpers::*;

pub const PROMPT_OBSERVATION_SCHEMA_VERSION: u32 = 2;

/// Why the session selected its base instructions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BaseInstructionsProvenance {
    ModelCatalog,
    ConfigOverride,
    RetainedSession,
}

impl BaseInstructionsProvenance {
    fn as_str(self) -> &'static str {
        match self {
            Self::ModelCatalog => "model_catalog",
            Self::ConfigOverride => "config_override",
            Self::RetainedSession => "retained_session",
        }
    }
}

/// Prompt components immediately before transport-specific request transformation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptAssemblyTracePayload {
    pub model: String,
    #[serde(default = "stock_prompt_profile")]
    pub prompt_profile: String,
    pub base_instructions: String,
    pub base_instructions_provenance: BaseInstructionsProvenance,
    pub retained_session_instructions: Option<String>,
    pub input: Vec<ResponseItem>,
    pub tools: Vec<Value>,
    pub responses_lite: bool,
    pub responses_lite_prefix_order: Vec<String>,
}

fn stock_prompt_profile() -> String {
    "stock".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptInventoryEntry {
    pub path: String,
    pub role: Option<String>,
    pub item_type: String,
    pub producing_subsystems: Vec<String>,
    pub provenance: String,
    pub character_count: usize,
    pub serialized_character_count: usize,
    pub estimated_tokens: usize,
    pub sha1: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptStableHashes {
    pub request_sha1: String,
    pub instructions_sha1: String,
    pub input_sha1: String,
    pub tools_sha1: String,
    pub pre_transform_input_sha1: Option<String>,
    pub pre_transform_tools_sha1: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponsesLiteObservation {
    pub enabled: bool,
    pub prefix_order: Vec<String>,
    pub instructions_moved_to_input: bool,
    pub tools_moved_to_input: bool,
    pub input_items_before: Option<usize>,
    pub input_items_after: usize,
    pub image_details_before: Option<usize>,
    pub image_details_after: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptRequestObservation {
    pub schema_version: u32,
    pub inference_call_id: String,
    pub model: String,
    pub prompt_profile: String,
    pub producing_subsystem: String,
    pub roles: Vec<String>,
    pub instructions: PromptInventoryEntry,
    pub ordered_input_items: Vec<PromptInventoryEntry>,
    pub serialized_tools: Vec<PromptInventoryEntry>,
    pub retained_session_instructions: Option<PromptInventoryEntry>,
    pub responses_lite_transformation: ResponsesLiteObservation,
    pub totals: PromptInventoryEntry,
    pub stable_hashes: PromptStableHashes,
}

/// Exact request plus deterministic diagnostic projections.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptRequestDebugView {
    pub request: Value,
    pub canonical_request: Value,
    pub observation: PromptRequestObservation,
}

impl PromptRequestDebugView {
    pub fn canonical_json(&self) -> Result<String> {
        serde_json::to_string(&self.canonical_request).context("serialize canonical request")
    }

    pub fn render_readable(&self) -> String {
        let observation = &self.observation;
        let mut lines = vec![
            format!("inference: {}", observation.inference_call_id),
            format!("model: {}", observation.model),
            format!("prompt_profile: {}", observation.prompt_profile),
            format!("producer: {}", observation.producing_subsystem),
            format!("roles: {}", observation.roles.join(", ")),
            format_entry("instructions", &observation.instructions),
            format!(
                "responses_lite: enabled={} prefix={} instructions_moved={} tools_moved={} image_details={}=>{}",
                observation.responses_lite_transformation.enabled,
                observation
                    .responses_lite_transformation
                    .prefix_order
                    .join(" -> "),
                observation
                    .responses_lite_transformation
                    .instructions_moved_to_input,
                observation
                    .responses_lite_transformation
                    .tools_moved_to_input,
                observation
                    .responses_lite_transformation
                    .image_details_before
                    .map_or_else(|| "unknown".to_string(), |count| count.to_string()),
                observation
                    .responses_lite_transformation
                    .image_details_after,
            ),
            format_entry("total", &observation.totals),
            format!("request_sha1: {}", observation.stable_hashes.request_sha1),
            "input:".to_string(),
        ];
        lines.extend(
            observation
                .ordered_input_items
                .iter()
                .map(|entry| format!("  {}", format_entry(&entry.path, entry))),
        );
        lines.push("tools:".to_string());
        lines.extend(
            observation
                .serialized_tools
                .iter()
                .map(|entry| format!("  {}", format_entry(&entry.path, entry))),
        );
        if let Some(retained) = &observation.retained_session_instructions {
            lines.push(format_entry("retained_session_instructions", retained));
        }
        lines.join("\n")
    }
}

pub fn load_prompt_request_debug_view(
    bundle_dir: &Path,
    inference_call_id: Option<&str>,
) -> Result<PromptRequestDebugView> {
    let trace = replay_bundle(bundle_dir)?;
    let inference = select_inference(&trace, inference_call_id)?;
    let request_payload_id = inference
        .raw_wire_request_payload_id
        .as_deref()
        .unwrap_or(&inference.raw_request_payload_id);
    let request = read_payload(bundle_dir, &trace, request_payload_id)?;
    let assembly = inference
        .raw_prompt_assembly_payload_id
        .as_deref()
        .map(|payload_id| read_payload(bundle_dir, &trace, payload_id))
        .transpose()?
        .map(serde_json::from_value::<PromptAssemblyTracePayload>)
        .transpose()
        .context("decode prompt assembly payload")?;
    Ok(observe_request(
        inference.inference_call_id.as_str(),
        request,
        assembly.as_ref(),
    ))
}

pub fn observe_request(
    inference_call_id: &str,
    request: Value,
    assembly: Option<&PromptAssemblyTracePayload>,
) -> PromptRequestDebugView {
    let canonical_request = canonicalize(request.clone());
    let model = request
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let instructions_value = request
        .get("instructions")
        .cloned()
        .or_else(|| assembly.map(|assembly| Value::String(assembly.base_instructions.clone())))
        .unwrap_or_else(|| Value::String(String::new()));
    let instructions_provenance = assembly
        .map(|assembly| assembly.base_instructions_provenance.as_str().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let instructions = inventory_entry(
        "instructions",
        None,
        "instructions",
        vec!["session::base_instructions".to_string()],
        instructions_provenance,
        &instructions_value,
    );
    let input = request
        .get("input")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let last_user_index = input
        .iter()
        .rposition(|item| item.get("role").and_then(Value::as_str) == Some("user"));
    let ordered_input_items = input
        .iter()
        .enumerate()
        .map(|(index, item)| inventory_input_item(index, item, last_user_index))
        .collect::<Vec<_>>();
    let roles = ordered_input_items
        .iter()
        .filter_map(|entry| entry.role.clone())
        .collect::<Vec<_>>();
    let tools = request
        .get("tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| additional_tools(&input));
    let serialized_tools = tools
        .iter()
        .enumerate()
        .map(|(index, tool)| {
            inventory_entry(
                format!("tools[{index}]").as_str(),
                None,
                "tool",
                vec!["core::tools::spec_plan".to_string()],
                "tool_registry".to_string(),
                tool,
            )
        })
        .collect::<Vec<_>>();
    let retained_session_instructions = assembly
        .and_then(|assembly| assembly.retained_session_instructions.as_ref())
        .map(|text| {
            inventory_entry(
                "retained_session_instructions",
                None,
                "instructions",
                vec!["session::resume".to_string()],
                "retained_session".to_string(),
                &Value::String(text.clone()),
            )
        });
    let responses_lite_transformation = responses_lite_observation(&input, assembly, &request);
    let totals = inventory_entry(
        "request",
        None,
        "responses_request",
        vec!["core::client".to_string()],
        "assembled_request".to_string(),
        &canonical_request,
    );
    let empty_array = Value::Array(Vec::new());
    let input_value = request.get("input").unwrap_or(&empty_array);
    let tools_value = request.get("tools").unwrap_or(&empty_array);
    let stable_hashes = PromptStableHashes {
        request_sha1: stable_hash(&canonical_request),
        instructions_sha1: stable_hash(&instructions_value),
        input_sha1: stable_hash(input_value),
        tools_sha1: stable_hash(tools_value),
        pre_transform_input_sha1: assembly
            .and_then(|assembly| serde_json::to_value(&assembly.input).ok())
            .map(|value| stable_hash(&value)),
        pre_transform_tools_sha1: assembly
            .and_then(|assembly| serde_json::to_value(&assembly.tools).ok())
            .map(|value| stable_hash(&value)),
    };
    PromptRequestDebugView {
        request,
        canonical_request,
        observation: PromptRequestObservation {
            schema_version: PROMPT_OBSERVATION_SCHEMA_VERSION,
            inference_call_id: inference_call_id.to_string(),
            model,
            prompt_profile: assembly
                .map(|assembly| assembly.prompt_profile.clone())
                .unwrap_or_else(stock_prompt_profile),
            producing_subsystem: "codex_core::client".to_string(),
            roles,
            instructions,
            ordered_input_items,
            serialized_tools,
            retained_session_instructions,
            responses_lite_transformation,
            totals,
            stable_hashes,
        },
    }
}

fn select_inference<'a>(
    trace: &'a RolloutTrace,
    inference_call_id: Option<&str>,
) -> Result<&'a InferenceCall> {
    if let Some(inference_call_id) = inference_call_id {
        return trace
            .inference_calls
            .get(inference_call_id)
            .with_context(|| format!("unknown inference call {inference_call_id}"));
    }
    trace
        .inference_calls
        .values()
        .max_by_key(|inference| inference.execution.started_seq)
        .context("trace contains no inference requests")
}

fn read_payload(bundle_dir: &Path, trace: &RolloutTrace, payload_id: &str) -> Result<Value> {
    let payload = trace
        .raw_payloads
        .get(payload_id)
        .with_context(|| format!("missing raw payload {payload_id}"))?;
    let path = bundle_dir.join(&payload.path);
    let bytes = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("decode {}", path.display()))
}

fn inventory_input_item(
    index: usize,
    item: &Value,
    last_user_index: Option<usize>,
) -> PromptInventoryEntry {
    let role = item.get("role").and_then(Value::as_str).map(str::to_string);
    let item_type = item
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let text = collect_text(item);
    let producing_subsystems = classify_subsystems(item_type.as_str(), role.as_deref(), &text);
    let provenance = match (item_type.as_str(), role.as_deref()) {
        ("additional_tools", _) => "responses_lite_transform",
        ("message", Some("developer")) => "generated_developer_context",
        ("message", Some("user")) if last_user_index == Some(index) && !looks_contextual(&text) => {
            "user_turn"
        }
        ("message", Some("user")) if looks_contextual(&text) => "generated_user_context",
        ("message", Some("user" | "assistant")) => "conversation_history",
        _ => "conversation_history",
    };
    inventory_entry(
        format!("input[{index}]").as_str(),
        role,
        item_type.as_str(),
        producing_subsystems,
        provenance.to_string(),
        item,
    )
}

fn responses_lite_observation(
    input: &[Value],
    assembly: Option<&PromptAssemblyTracePayload>,
    request: &Value,
) -> ResponsesLiteObservation {
    let enabled = assembly
        .map(|assembly| assembly.responses_lite)
        .unwrap_or_else(|| {
            request.get("tools").is_none()
                && input.iter().any(|item| {
                    item.get("type").and_then(Value::as_str) == Some("additional_tools")
                })
        });
    let prefix_order = input
        .iter()
        .take(2)
        .filter_map(|item| {
            let item_type = item.get("type").and_then(Value::as_str)?;
            if item_type == "additional_tools" {
                Some("tools".to_string())
            } else if item.get("role").and_then(Value::as_str) == Some("developer") {
                Some("base_instructions".to_string())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    ResponsesLiteObservation {
        enabled,
        prefix_order,
        instructions_moved_to_input: enabled
            && request
                .get("instructions")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
            && assembly.is_some_and(|assembly| !assembly.base_instructions.is_empty()),
        tools_moved_to_input: enabled
            && request.get("tools").is_none()
            && input
                .iter()
                .any(|item| item.get("type").and_then(Value::as_str) == Some("additional_tools")),
        input_items_before: assembly.map(|assembly| assembly.input.len()),
        input_items_after: input.len(),
        image_details_before: assembly.and_then(|assembly| {
            serde_json::to_value(&assembly.input)
                .ok()
                .map(|value| count_image_details(&value))
        }),
        image_details_after: count_image_details(&Value::Array(input.to_vec())),
    }
}

fn inventory_entry(
    path: &str,
    role: Option<String>,
    item_type: &str,
    producing_subsystems: Vec<String>,
    provenance: String,
    value: &Value,
) -> PromptInventoryEntry {
    let canonical = canonicalize(value.clone());
    let serialized = serde_json::to_string(&canonical).unwrap_or_default();
    let text = collect_text(value);
    let character_count = text.chars().count();
    let serialized_character_count = serialized.chars().count();
    PromptInventoryEntry {
        path: path.to_string(),
        role,
        item_type: item_type.to_string(),
        producing_subsystems,
        provenance,
        character_count,
        serialized_character_count,
        estimated_tokens: serialized_character_count.saturating_add(3) / 4,
        sha1: stable_hash(&canonical),
    }
}

#[cfg(test)]
#[path = "prompt_observation_tests.rs"]
mod tests;

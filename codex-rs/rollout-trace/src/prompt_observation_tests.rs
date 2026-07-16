use codex_protocol::models::ResponseItem;
use pretty_assertions::assert_eq;
use serde_json::Value;
use serde_json::json;

use super::BaseInstructionsProvenance;
use super::PromptAssemblyTracePayload;
use super::observe_request;

#[test]
fn responses_lite_inventory_exposes_transform_and_retained_instructions() {
    let assembly_input = vec![response_item(json!({
        "type": "message",
        "role": "user",
        "content": [{"type": "input_text", "text": "hello"}]
    }))];
    let tools = vec![json!({
        "type": "function",
        "name": "shell",
        "description": "Run a shell command",
        "parameters": {"type": "object", "properties": {}}
    })];
    let assembly = PromptAssemblyTracePayload {
        model: "gpt-test".to_string(),
        prompt_profile: "stock".to_string(),
        base_instructions: "base instructions".to_string(),
        base_instructions_provenance: BaseInstructionsProvenance::RetainedSession,
        retained_session_instructions: Some("base instructions".to_string()),
        input: assembly_input,
        tools: tools.clone(),
        responses_lite: true,
        responses_lite_prefix_order: vec!["tools".to_string(), "base_instructions".to_string()],
    };
    let request = json!({
        "model": "gpt-test",
        "input": [
            {"type": "additional_tools", "role": "developer", "tools": tools},
            {
                "type": "message",
                "role": "developer",
                "content": [{"type": "input_text", "text": "base instructions"}]
            },
            {
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "hello"}]
            }
        ],
        "tool_choice": "auto",
        "parallel_tool_calls": false
    });

    let view = observe_request("inference-1", request, Some(&assembly));

    assert_eq!(view.observation.model, "gpt-test");
    assert_eq!(
        view.observation.responses_lite_transformation.prefix_order,
        vec!["tools", "base_instructions"]
    );
    assert!(
        view.observation
            .responses_lite_transformation
            .instructions_moved_to_input
    );
    assert!(
        view.observation
            .responses_lite_transformation
            .tools_moved_to_input
    );
    assert_eq!(
        view.observation
            .retained_session_instructions
            .as_ref()
            .map(|entry| entry.provenance.as_str()),
        Some("retained_session")
    );
    assert_eq!(
        view.observation
            .ordered_input_items
            .last()
            .map(|entry| entry.provenance.as_str()),
        Some("user_turn")
    );
    assert!(view.render_readable().contains("request_sha1:"));
}

#[test]
fn canonical_json_and_hash_ignore_object_key_order() {
    let left: Value =
        serde_json::from_str(r#"{"model":"gpt-test","instructions":"base","input":[],"tools":[]}"#)
            .expect("left request");
    let right: Value =
        serde_json::from_str(r#"{"tools":[],"input":[],"instructions":"base","model":"gpt-test"}"#)
            .expect("right request");

    let left = observe_request("left", left, None);
    let right = observe_request("right", right, None);

    assert_eq!(
        left.canonical_json().expect("left canonical JSON"),
        right.canonical_json().expect("right canonical JSON")
    );
    assert_eq!(
        left.observation.stable_hashes.request_sha1,
        right.observation.stable_hashes.request_sha1
    );
}

fn response_item(value: Value) -> ResponseItem {
    serde_json::from_value(value).expect("response item")
}

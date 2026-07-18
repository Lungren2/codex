use std::path::PathBuf;

use pretty_assertions::assert_eq;
use serde_json::json;

use crate::models::ValidationStatus;

use super::{
    AppServerError, absolute_utf8_path, extract_turn_evidence, is_server_request, rpc_result,
    string_at,
};

#[test]
fn response_helpers_preserve_wire_errors_and_nested_ids() {
    assert_eq!(
        string_at(
            &json!({"thread": {"id": "thread-1"}}),
            &["thread", "id"],
            "thread/start"
        )
        .expect("nested id"),
        "thread-1"
    );
    let error = rpc_result(
        "thread/start",
        json!({"error": {"code": -1, "message": "denied"}}),
    )
    .expect_err("RPC error");
    assert!(
        matches!(error, AppServerError::Rpc { method, message } if method == "thread/start" && message == "denied")
    );
}

#[test]
fn only_messages_with_a_method_and_id_are_server_requests() {
    assert!(is_server_request(
        &json!({"id": 7, "method": "item/request"})
    ));
    assert!(!is_server_request(&json!({"method": "turn/completed"})));
    assert!(!is_server_request(&json!({"id": 7, "result": {}})));
}

#[test]
fn synthesis_workspace_must_be_absolute() {
    let relative = PathBuf::from("patches");
    assert!(absolute_utf8_path(&relative).is_err());
}

#[test]
fn extracts_completed_turn_messages_and_command_evidence() {
    let response = serde_json::json!({
        "thread": { "turns": [{
            "id": "turn-1",
            "status": "completed",
            "items": [
                {"type":"userMessage","content":[{"type":"text","text":"Fix it"}]},
                {"type":"commandExecution","command":"pnpm test","status":"completed","exitCode":0,"aggregatedOutput":"4 passed"},
                {"type":"agentMessage","text":"Implemented and tested."}
            ]
        }]}
    });
    let evidence = extract_turn_evidence(&response, "turn-1").unwrap();
    assert_eq!(evidence.status, "completed");
    assert_eq!(evidence.original_request, "Fix it");
    assert_eq!(evidence.final_response, "Implemented and tested.");
    assert_eq!(evidence.validations.len(), 1);
    assert_eq!(evidence.validations[0].status, ValidationStatus::Passed);
}

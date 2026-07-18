use super::*;
use pretty_assertions::assert_eq;

#[test]
fn parses_legacy_kebab_case_payload() {
    let payload = CompletionPayload::parse(r#"{"type":"agent-turn-complete","thread-id":"thread","turn-id":"turn","cwd":"C:\\repo","input-messages":["do it"],"last-assistant-message":"done"}"#).unwrap();
    assert_eq!(payload.thread_id, "thread");
    assert_eq!(payload.turn_id, "turn");
    assert_eq!(payload.input_messages, vec!["do it"]);
    assert!(payload.should_capture());
}

#[test]
fn filters_failed_and_non_desktop_completions() {
    let payload = CompletionPayload::parse(r#"{"type":"agent-turn-complete","threadId":"thread","turnId":"turn","cwd":"C:\\repo","status":"failed","origin":"desktop"}"#).unwrap();
    assert!(!payload.should_capture());
}

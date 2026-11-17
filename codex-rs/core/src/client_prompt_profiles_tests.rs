use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn responses_lite_request_retains_pre_transform_prompt_debug_data() -> anyhow::Result<()> {
    let retained_instructions = "retained session instructions";
    let client = test_model_client(SessionSource::Cli).with_prompt_debug_metadata(
        BaseInstructionsProvenance::RetainedSession,
        Some(retained_instructions.to_string()),
    );
    let prompt = Prompt {
        input: vec![ResponseItem::Message {
            encrypted_content: None,
            status: None,
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "hello".to_string(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: None,
        }],
        base_instructions: BaseInstructions {
            text: retained_instructions.to_string(),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut model_info = test_model_info();
    model_info.use_responses_lite = true;
    let responses_metadata = test_responses_metadata_for_client(
        &client,
        /*turn_id*/ None,
        format!("{}:0", client.state.thread_id),
        /*parent_thread_id*/ None,
        TestCodexResponsesRequestKind::Turn,
    );

    let super::super::BuiltResponsesRequest {
        request,
        prompt_assembly,
    } = client.build_responses_request_with_prompt_assembly(
        &prompt,
        &model_info,
        /*effort*/ None,
        codex_protocol::config_types::ReasoningSummary::None,
        /*service_tier*/ None,
        &responses_metadata,
        /*include_internal*/ true,
    )?;

    assert_eq!(
        prompt_assembly,
        codex_rollout_trace::PromptAssemblyTracePayload {
            model: "gpt-test".to_string(),
            prompt_profile: "stock".to_string(),
            base_instructions: retained_instructions.to_string(),
            base_instructions_provenance: BaseInstructionsProvenance::RetainedSession,
            retained_session_instructions: Some(retained_instructions.to_string()),
            input: prompt.input.clone(),
            tools: Vec::new(),
            responses_lite: true,
            responses_lite_prefix_order: vec!["base_instructions".to_string()],
        }
    );
    assert!(
        serde_json::to_value(&request)?
            .get("instructions")
            .is_none()
    );
    assert!(request.tools.is_none());
    assert_eq!(
        request
            .input
            .iter()
            .map(|item| match item {
                ResponseItem::AdditionalTools { role, .. } | ResponseItem::Message { role, .. } =>
                    Some(role.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![Some("developer"), Some("user")]
    );

    let empty_base_prompt = Prompt {
        base_instructions: BaseInstructions {
            text: String::new(),
            ..Default::default()
        },
        ..prompt.clone()
    };
    let empty_base = client.build_responses_request_with_prompt_assembly(
        &empty_base_prompt,
        &model_info,
        /*effort*/ None,
        codex_protocol::config_types::ReasoningSummary::None,
        /*service_tier*/ None,
        &responses_metadata,
        /*include_internal*/ true,
    )?;
    assert_eq!(
        empty_base.prompt_assembly.responses_lite_prefix_order,
        Vec::<String>::new()
    );
    assert!(matches!(
        empty_base.request.input.as_slice(),
        [ResponseItem::Message { role, .. }]
            if role == "user"
    ));

    Ok(())
}

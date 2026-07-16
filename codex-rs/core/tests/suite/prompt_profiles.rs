use anyhow::Context;
use anyhow::Result;
use codex_config::types::ExperimentalPromptProfile;
use codex_protocol::config_types::CollaborationMode;
use codex_protocol::config_types::ModeKind;
use codex_protocol::config_types::Settings;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::user_input::UserInput;
use core_test_support::responses;
use core_test_support::skip_if_no_network;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use pretty_assertions::assert_eq;
use serde_json::Value;

const STOCK_BASE: &str = "Base start.\n## Personality\nRemove this tone.\n## Planning\nKeep this harness guidance.\n## Presenting your work and final message\nRemove this renderer guidance.";
const USER_REQUEST: &str = "preserve this exact user request";
const DEVELOPER_CONTEXT: &str = "preserve this configured developer context";

struct RequestSnapshot {
    instructions: String,
    input: Vec<Value>,
    tools: Vec<Value>,
}

async fn request_for_profile(
    profile: Option<ExperimentalPromptProfile>,
) -> Result<RequestSnapshot> {
    let server = responses::start_mock_server().await;
    let response_mock = responses::mount_sse_once(
        &server,
        responses::sse(vec![
            responses::ev_response_created("resp-1"),
            responses::ev_completed("resp-1"),
        ]),
    )
    .await;
    let fixed_cwd = codex_utils_absolute_path::AbsolutePathBuf::from_absolute_path(
        std::env::current_dir().context("current directory")?,
    )
    .context("absolute current directory")?;
    let mut builder = test_codex()
        .with_model_info_override("gpt-5.4", |model_info| {
            model_info.base_instructions = STOCK_BASE.to_string();
            model_info.model_messages = None;
        })
        .with_config(move |config| {
            if let Some(profile) = profile {
                config.experimental_prompt_profile = profile;
            }
            config.cwd = fixed_cwd;
            config.developer_instructions = Some(DEVELOPER_CONTEXT.to_string());
            config.include_apps_instructions = false;
            config.include_skill_instructions = false;
        });
    let test = builder.build_with_auto_env(&server).await?;
    let stock_collaboration =
        codex_models_manager::collaboration_mode_presets::builtin_collaboration_mode_presets()
            .into_iter()
            .find(|preset| preset.mode == Some(ModeKind::Default))
            .and_then(|preset| preset.developer_instructions)
            .flatten()
            .context("stock default collaboration instructions")?;
    test.codex
        .submit(Op::UserInput {
            items: vec![UserInput::Text {
                text: USER_REQUEST.to_string(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
            additional_context: Default::default(),
            thread_settings: codex_protocol::protocol::ThreadSettingsOverrides {
                collaboration_mode: Some(CollaborationMode {
                    mode: ModeKind::Default,
                    settings: Settings {
                        model: "gpt-5.4".to_string(),
                        reasoning_effort: None,
                        developer_instructions: Some(stock_collaboration),
                    },
                }),
                ..Default::default()
            },
        })
        .await?;
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    let request = response_mock.single_request();
    let body = request.body_json();
    Ok(RequestSnapshot {
        instructions: body
            .get("instructions")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        input: request.input(),
        tools: body
            .get("tools")
            .and_then(Value::as_array)
            .context("request tools")?
            .clone(),
    })
}

fn message_input_texts<'a>(input: &'a [Value], role: &str) -> Vec<&'a str> {
    input
        .iter()
        .filter(|item| item.get("role").and_then(Value::as_str) == Some(role))
        .filter_map(|item| item.get("content").and_then(Value::as_array))
        .flatten()
        .filter_map(|content| content.get("text").and_then(Value::as_str))
        .collect()
}

fn without_profile_fragments(input: &[Value]) -> Vec<Value> {
    without_internal_metadata(input.to_vec())
        .into_iter()
        .filter(|item| {
            let serialized = item.to_string();
            !serialized.contains("<permissions instructions>")
                && !serialized.contains("<permissions_instructions>")
                && !serialized.contains("Effective permissions:")
                && !serialized.contains("<collaboration_mode>")
                && !serialized.contains("# Personality")
                && !serialized.contains("<recommended_plugins>")
        })
        .collect()
}

fn without_tool_descriptions(mut tools: Vec<Value>) -> Vec<Value> {
    for tool in &mut tools {
        remove_tool_descriptions(tool);
    }
    tools
}

fn remove_tool_descriptions(tool: &mut Value) {
    let Some(tool) = tool.as_object_mut() else {
        return;
    };
    tool.remove("description");
    if let Some(function) = tool.get_mut("function") {
        remove_tool_descriptions(function);
    }
    if let Some(Value::Array(nested_tools)) = tool.get_mut("tools") {
        for nested_tool in nested_tools {
            remove_tool_descriptions(nested_tool);
        }
    }
}

fn without_internal_metadata(mut input: Vec<Value>) -> Vec<Value> {
    for item in &mut input {
        if let Some(item) = item.as_object_mut() {
            item.remove("internal_chat_message_metadata_passthrough");
        }
    }
    input
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prompt_profiles_change_only_their_declared_request_contributions() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let unconfigured = request_for_profile(None).await?;
    let stock = request_for_profile(Some(ExperimentalPromptProfile::Stock)).await?;
    assert_eq!(stock.instructions, unconfigured.instructions);
    assert_eq!(
        without_internal_metadata(stock.input.clone()),
        without_internal_metadata(unconfigured.input)
    );
    assert_eq!(stock.tools, unconfigured.tools);
    assert_eq!(stock.instructions, STOCK_BASE);
    assert!(!stock.tools.is_empty());

    for profile in [
        ExperimentalPromptProfile::MinimalBase,
        ExperimentalPromptProfile::NeutralLength,
        ExperimentalPromptProfile::NoProductGuidance,
        ExperimentalPromptProfile::CompactPermissions,
        ExperimentalPromptProfile::CompactCollaboration,
        ExperimentalPromptProfile::CompactTools,
        ExperimentalPromptProfile::FullyMinimal,
    ] {
        let request = request_for_profile(Some(profile)).await?;
        assert!(message_input_texts(&request.input, "user").contains(&USER_REQUEST));
        assert!(message_input_texts(&request.input, "developer").contains(&DEVELOPER_CONTEXT));

        match profile {
            ExperimentalPromptProfile::MinimalBase => {
                assert!(request.instructions.contains("coding agent"));
                assert_ne!(request.instructions, stock.instructions);
                assert_eq!(
                    without_internal_metadata(request.input.clone()),
                    without_internal_metadata(stock.input.clone())
                );
                assert_eq!(request.tools, stock.tools);
            }
            ExperimentalPromptProfile::NeutralLength => {
                assert_ne!(request.instructions, stock.instructions);
                assert_eq!(
                    request.instructions.chars().count(),
                    stock.instructions.chars().count()
                );
                assert_eq!(
                    without_internal_metadata(request.input.clone()),
                    without_internal_metadata(stock.input.clone())
                );
                assert_eq!(request.tools, stock.tools);
            }
            ExperimentalPromptProfile::NoProductGuidance => {
                assert!(!request.instructions.contains("Personality"));
                assert!(!request.instructions.contains("renderer guidance"));
                assert!(request.instructions.contains("Keep this harness guidance"));
                assert_eq!(
                    without_profile_fragments(&request.input),
                    without_profile_fragments(&stock.input)
                );
                assert_eq!(request.tools, stock.tools);
            }
            ExperimentalPromptProfile::CompactPermissions => {
                assert_eq!(request.instructions, stock.instructions);
                assert_eq!(request.tools, stock.tools);
                assert!(
                    request
                        .input
                        .iter()
                        .any(|item| { item.to_string().contains("Effective permissions:") })
                );
                assert_eq!(
                    without_profile_fragments(&request.input),
                    without_profile_fragments(&stock.input)
                );
            }
            ExperimentalPromptProfile::CompactCollaboration => {
                assert_eq!(request.instructions, stock.instructions);
                assert_eq!(request.tools, stock.tools);
                let collaboration_texts = message_input_texts(&request.input, "developer")
                    .into_iter()
                    .filter(|text| text.contains("collaboration_mode"))
                    .collect::<Vec<_>>();
                assert!(
                    collaboration_texts
                        .iter()
                        .any(|text| text.contains("Default mode. Execute in-scope requests")),
                    "observed collaboration texts: {collaboration_texts:#?}"
                );
                assert_eq!(
                    without_profile_fragments(&request.input),
                    without_profile_fragments(&stock.input)
                );
            }
            ExperimentalPromptProfile::CompactTools => {
                assert_eq!(request.instructions, stock.instructions);
                assert_eq!(
                    without_internal_metadata(request.input.clone()),
                    without_internal_metadata(stock.input.clone())
                );
                assert_eq!(
                    without_tool_descriptions(request.tools.clone()),
                    without_tool_descriptions(stock.tools.clone())
                );
                assert_ne!(request.tools, stock.tools);
            }
            ExperimentalPromptProfile::FullyMinimal => {
                assert!(request.instructions.contains("coding agent"));
                assert!(
                    request
                        .input
                        .iter()
                        .any(|item| { item.to_string().contains("Effective permissions:") })
                );
                assert_eq!(
                    without_tool_descriptions(request.tools.clone()),
                    without_tool_descriptions(stock.tools.clone())
                );
                assert_eq!(
                    without_profile_fragments(&request.input),
                    without_profile_fragments(&stock.input)
                );
            }
            ExperimentalPromptProfile::Stock => unreachable!(),
        }
    }
    Ok(())
}

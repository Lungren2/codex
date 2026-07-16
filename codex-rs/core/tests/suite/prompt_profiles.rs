use anyhow::Context;
use anyhow::Result;
use codex_config::types::ExperimentalPromptProfile;
use codex_protocol::config_types::CollaborationMode;
use codex_protocol::config_types::ModeKind;
use codex_protocol::config_types::Settings;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::user_input::UserInput;
use core_test_support::responses;
use core_test_support::skip_if_no_network;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use pretty_assertions::assert_eq;
use serde_json::Value;
use test_case::test_case;

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
    let mut builder = test_codex()
        .with_model_info_override("gpt-5.4", |model_info| {
            model_info.base_instructions = STOCK_BASE.to_string();
            model_info.model_messages = None;
        })
        .with_config(move |config| {
            if let Some(profile) = profile {
                config.experimental_prompt_profile = profile;
            }
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
    let snapshot = RequestSnapshot {
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
    };
    Ok(snapshot)
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unconfigured_prompt_profile_uses_stock_request() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let unconfigured = request_for_profile(None).await?;
    assert_eq!(unconfigured.instructions, STOCK_BASE);
    assert!(message_input_texts(&unconfigured.input, "user").contains(&USER_REQUEST));
    assert!(message_input_texts(&unconfigured.input, "developer").contains(&DEVELOPER_CONTEXT));
    assert!(!unconfigured.tools.is_empty());
    Ok(())
}

#[test_case(ExperimentalPromptProfile::Stock; "stock")]
#[test_case(ExperimentalPromptProfile::MinimalBase; "minimal base")]
#[test_case(ExperimentalPromptProfile::NeutralLength; "neutral length")]
#[test_case(ExperimentalPromptProfile::NoProductGuidance; "no product guidance")]
#[test_case(ExperimentalPromptProfile::CompactPermissions; "compact permissions")]
#[test_case(ExperimentalPromptProfile::CompactCollaboration; "compact collaboration")]
#[test_case(ExperimentalPromptProfile::CompactTools; "compact tools")]
#[test_case(ExperimentalPromptProfile::FullyMinimal; "fully minimal")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prompt_profile_changes_only_its_declared_request_contributions(
    profile: ExperimentalPromptProfile,
) -> Result<()> {
    skip_if_no_network!(Ok(()));

    let request = request_for_profile(Some(profile)).await?;
    assert!(message_input_texts(&request.input, "user").contains(&USER_REQUEST));
    assert!(message_input_texts(&request.input, "developer").contains(&DEVELOPER_CONTEXT));

    match profile {
        ExperimentalPromptProfile::Stock => {
            assert_eq!(request.instructions, STOCK_BASE);
            assert!(!request.tools.is_empty());
        }
        ExperimentalPromptProfile::MinimalBase => {
            assert!(request.instructions.contains("coding agent"));
            assert_ne!(request.instructions, STOCK_BASE);
            assert!(!request.tools.is_empty());
        }
        ExperimentalPromptProfile::NeutralLength => {
            assert_ne!(request.instructions, STOCK_BASE);
            assert_eq!(
                request.instructions.chars().count(),
                STOCK_BASE.chars().count()
            );
            assert!(!request.tools.is_empty());
        }
        ExperimentalPromptProfile::NoProductGuidance => {
            assert!(!request.instructions.contains("Personality"));
            assert!(!request.instructions.contains("renderer guidance"));
            assert!(request.instructions.contains("Keep this harness guidance"));
            assert!(!request.tools.is_empty());
        }
        ExperimentalPromptProfile::CompactPermissions => {
            assert_eq!(request.instructions, STOCK_BASE);
            assert!(!request.tools.is_empty());
            assert!(
                request
                    .input
                    .iter()
                    .any(|item| { item.to_string().contains("Effective permissions:") })
            );
        }
        ExperimentalPromptProfile::CompactCollaboration => {
            assert_eq!(request.instructions, STOCK_BASE);
            assert!(!request.tools.is_empty());
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
        }
        ExperimentalPromptProfile::CompactTools => {
            assert_eq!(request.instructions, STOCK_BASE);
            assert!(!request.tools.is_empty());
            assert!(request.tools.iter().any(|tool| {
                tool.get("description")
                    .and_then(Value::as_str)
                    .is_some_and(|description| description.len() < 300)
            }));
        }
        ExperimentalPromptProfile::FullyMinimal => {
            assert!(request.instructions.contains("coding agent"));
            assert!(
                request
                    .input
                    .iter()
                    .any(|item| { item.to_string().contains("Effective permissions:") })
            );
            assert!(!request.tools.is_empty());
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fully_minimal_profiles_permission_and_collaboration_updates() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = responses::start_mock_server().await;
    let response_mock = responses::mount_sse_sequence(
        &server,
        vec![
            responses::sse(vec![
                responses::ev_response_created("resp-1"),
                responses::ev_completed("resp-1"),
            ]),
            responses::sse(vec![
                responses::ev_response_created("resp-2"),
                responses::ev_completed("resp-2"),
            ]),
        ],
    )
    .await;
    let mut builder = test_codex().with_config(|config| {
        config.experimental_prompt_profile = ExperimentalPromptProfile::FullyMinimal;
        config.include_apps_instructions = false;
        config.include_skill_instructions = false;
    });
    let test = builder.build_with_auto_env(&server).await?;
    let stock_instructions_for = |mode| {
        codex_models_manager::collaboration_mode_presets::builtin_collaboration_mode_presets()
            .into_iter()
            .find(|preset| preset.mode == Some(mode))
            .and_then(|preset| preset.developer_instructions)
            .flatten()
            .expect("stock collaboration instructions")
    };

    for (index, mode) in [ModeKind::Default, ModeKind::Plan].into_iter().enumerate() {
        test.codex
            .submit(Op::UserInput {
                items: vec![UserInput::Text {
                    text: format!("turn {index}"),
                    text_elements: Vec::new(),
                }],
                final_output_json_schema: None,
                responsesapi_client_metadata: None,
                additional_context: Default::default(),
                thread_settings: codex_protocol::protocol::ThreadSettingsOverrides {
                    approval_policy: (mode == ModeKind::Plan).then_some(AskForApproval::Never),
                    collaboration_mode: Some(CollaborationMode {
                        mode,
                        settings: Settings {
                            model: "gpt-5.4".to_string(),
                            reasoning_effort: None,
                            developer_instructions: Some(stock_instructions_for(mode)),
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
    }

    let requests = response_mock.requests();
    assert_eq!(requests.len(), 2);
    let initial_developer = requests[0].message_input_texts("developer").join("\n");
    assert!(initial_developer.contains("<collaboration_mode>Default mode."));
    assert!(initial_developer.contains("Effective permissions:"));
    let updated_developer = requests[1].message_input_texts("developer").join("\n");
    assert!(updated_developer.contains("<collaboration_mode>Plan mode."));
    assert!(updated_developer.contains("approval_policy=never"));
    assert!(!updated_developer.contains("# Collaboration Mode:"));
    Ok(())
}

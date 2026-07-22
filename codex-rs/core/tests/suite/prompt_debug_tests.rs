use std::sync::Arc;

use anyhow::Result;
use codex_core::ModelClient;
use codex_core::Prompt;
use codex_core::ResponseEvent;
use codex_core::build_prompt_input;
use codex_core::config::ConfigBuilder;
use codex_core::config::ConfigOverrides;
use codex_home::CodexHomeUserInstructionsProvider;
use codex_login::auth::AgentIdentityAuthPolicy;
use codex_model_provider_info::ModelProviderInfo;
use codex_model_provider_info::WireApi;
use codex_otel::SessionTelemetry;
use codex_protocol::ThreadId;
use codex_protocol::config_types::ReasoningSummary;
use codex_protocol::models::BaseInstructions;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::SessionSource;
use codex_protocol::user_input::UserInput;
use codex_rollout_trace::InferenceTraceContext;
use codex_rollout_trace::RawTraceEventPayload;
use codex_rollout_trace::TraceWriter;
use codex_rollout_trace::load_prompt_request_debug_view;
use core_test_support::TestCodexResponsesRequestKind;
use core_test_support::load_default_config_for_test;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::mount_sse_once;
use core_test_support::responses::sse;
use core_test_support::responses::strip_metadata;
use core_test_support::responses::strip_response_item_id;
use core_test_support::responses_metadata;
use core_test_support::skip_if_no_network;
use futures::StreamExt;
use pretty_assertions::assert_eq;
use tempfile::TempDir;

const TEST_INSTRUCTIONS: &str = "Global test instructions";
const TEST_INSTALLATION_ID: &str = "11111111-1111-4111-8111-111111111111";
const TEST_WINDOW_ID: &str = "test-thread:0";

#[tokio::test]
async fn build_prompt_input_includes_context_and_user_message() -> Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;
    std::fs::write(codex_home.path().join("AGENTS.md"), TEST_INSTRUCTIONS)?;
    let config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .harness_overrides(ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            codex_self_exe: Some(std::env::current_exe()?),
            ..ConfigOverrides::default()
        })
        .build()
        .await?;
    let user_instructions_provider = Arc::new(CodexHomeUserInstructionsProvider::new(
        config.codex_home.clone(),
    ));

    let input = build_prompt_input(
        config,
        vec![UserInput::Text {
            text: "hello from debug prompt".to_string(),
            text_elements: Vec::new(),
        }],
        /*state_db*/ None,
        user_instructions_provider,
    )
    .await?;

    let expected_user_message = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "hello from debug prompt".to_string(),
        }],
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    };
    assert_eq!(
        input
            .last()
            .cloned()
            .map(strip_metadata)
            .map(strip_response_item_id),
        Some(expected_user_message)
    );
    assert!(input.iter().any(|item| {
        let ResponseItem::Message { content, .. } = item else {
            return false;
        };

        content.iter().any(|content_item| {
            let (ContentItem::InputText { text } | ContentItem::OutputText { text }) = content_item
            else {
                return false;
            };
            text.contains(TEST_INSTRUCTIONS)
        })
    }));

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_responses_trace_loads_exact_request_debug_view() {
    skip_if_no_network!();

    let server = wiremock::MockServer::start().await;
    let response_mock = mount_sse_once(
        &server,
        sse(vec![
            ev_response_created("resp-trace"),
            ev_completed("resp-trace"),
        ]),
    )
    .await;
    let provider = ModelProviderInfo {
        name: "trace-test".into(),
        base_url: Some(format!("{}/v1", server.uri())),
        env_key: None,
        env_key_instructions: None,
        experimental_bearer_token: None,
        auth: None,
        aws: None,
        wire_api: WireApi::Responses,
        query_params: None,
        http_headers: None,
        env_http_headers: None,
        request_max_retries: Some(0),
        stream_max_retries: Some(0),
        stream_idle_timeout_ms: Some(5_000),
        websocket_connect_timeout_ms: None,
        requires_openai_auth: false,
        supports_websockets: false,
    };
    let codex_home = TempDir::new().expect("create codex home");
    let config = Arc::new(load_default_config_for_test(&codex_home).await);
    let model = codex_core::test_support::get_model_offline(config.model.as_deref());
    let model_info = codex_core::test_support::construct_model_info_offline(&model, &config);
    let thread_id = ThreadId::new();
    let session_telemetry = SessionTelemetry::new(
        thread_id,
        model.as_str(),
        model_info.slug.as_str(),
        /*account_id*/ None,
        Some("test@test.com".to_string()),
        /*auth_mode*/ None,
        "test_originator".to_string(),
        /*log_user_prompts*/ false,
        "test".to_string(),
        SessionSource::Exec,
    );
    let client = ModelClient::new(
        /*auth_manager*/ None,
        AgentIdentityAuthPolicy::JwtOnly,
        thread_id,
        provider,
        SessionSource::Exec,
        "test_originator".to_string(),
        config.model_verbosity,
        /*enable_request_compression*/ false,
        /*include_timing_metrics*/ false,
        /*beta_features_header*/ None,
        /*item_ids_enabled*/ false,
        /*concurrent_reasoning_summaries_enabled*/ false,
        /*attestation_provider*/ None,
        config.http_client_factory(),
    );
    let thread_id = thread_id.to_string();
    let responses_metadata = responses_metadata(
        TEST_INSTALLATION_ID,
        &thread_id,
        &thread_id,
        /*turn_id*/ None,
        TEST_WINDOW_ID.to_string(),
        &SessionSource::Exec,
        /*parent_thread_id*/ None,
        TestCodexResponsesRequestKind::Turn,
    );
    let prompt = Prompt {
        input: vec![ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "trace this request".to_string(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: None,
        }],
        base_instructions: BaseInstructions {
            text: "trace base instructions".to_string(),
        },
        ..Default::default()
    };
    let trace_dir = TempDir::new().expect("create trace dir");
    let writer = Arc::new(
        TraceWriter::create(
            trace_dir.path(),
            "trace-1".to_string(),
            "rollout-1".to_string(),
            thread_id.clone(),
        )
        .expect("create trace writer"),
    );
    writer
        .append(RawTraceEventPayload::ThreadStarted {
            thread_id: thread_id.clone(),
            agent_path: "/root".to_string(),
            metadata_payload: None,
        })
        .expect("record thread start");
    writer
        .append(RawTraceEventPayload::CodexTurnStarted {
            codex_turn_id: "turn-1".to_string(),
            thread_id: thread_id.clone(),
        })
        .expect("record turn start");
    let inference_trace = InferenceTraceContext::enabled(
        writer,
        thread_id,
        "turn-1".to_string(),
        model_info.slug.clone(),
        "trace-test".to_string(),
    );

    let mut client_session = client.new_session();
    let mut stream = client_session
        .stream(
            &prompt,
            &model_info,
            &session_telemetry,
            config.model_reasoning_effort.clone(),
            config
                .model_reasoning_summary
                .unwrap_or(ReasoningSummary::Auto),
            /*service_tier*/ None,
            &responses_metadata,
            &inference_trace,
        )
        .await
        .expect("responses stream to start");
    while let Some(event) = stream.next().await {
        if matches!(event, Ok(ResponseEvent::Completed { .. })) {
            break;
        }
    }

    let request = response_mock.single_request().body_json();
    let debug_view = load_prompt_request_debug_view(trace_dir.path(), None)
        .expect("load prompt request debug view from HTTP trace");
    assert_eq!(debug_view.canonical_request, request);
    assert_eq!(
        (
            debug_view.observation.instructions.character_count,
            debug_view.observation.instructions.provenance.as_str(),
            debug_view.observation.ordered_input_items.len(),
            debug_view.observation.stable_hashes.request_sha1.len(),
            debug_view.observation.stable_hashes.instructions_sha1.len(),
            debug_view.observation.stable_hashes.input_sha1.len(),
            debug_view.observation.stable_hashes.tools_sha1.len(),
        ),
        (23, "model_catalog", 1, 40, 40, 40, 40)
    );
}

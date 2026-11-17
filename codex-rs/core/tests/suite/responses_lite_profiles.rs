use super::*;
use codex_config::types::ExperimentalResponsesLitePrefixOrder;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn responses_lite_uses_input_items_for_instructions_and_tools() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let fixed_cwd = codex_utils_absolute_path::AbsolutePathBuf::from_absolute_path(
        std::env::current_dir().context("current directory")?,
    )
    .context("absolute current directory")?;
    let mut canonical_tools = None;
    for order in [
        ExperimentalResponsesLitePrefixOrder::ToolsThenBase,
        ExperimentalResponsesLitePrefixOrder::BaseThenTools,
    ] {
        let server = responses::start_mock_server().await;
        let response_mock = responses::mount_sse_once(
            &server,
            responses::sse(vec![
                responses::ev_response_created("resp-1"),
                responses::ev_completed("resp-1"),
            ]),
        )
        .await;

        let configured_cwd = fixed_cwd.clone();
        let mut builder = test_codex()
            .with_model_info_override("gpt-5.4", |model_info| {
                model_info.use_responses_lite = true;
            })
            .with_config(move |config| {
                config.cwd = configured_cwd;
                config.base_instructions = Some("test instructions".to_string());
                config.experimental_responses_lite_prefix_order = order;
            });
        let test = builder.build(&server).await?;

        test.submit_turn("hello").await?;

        let mut body = response_mock.single_request().body_json();
        assert!(body.get("instructions").is_none());
        assert!(body.get("tools").is_none());

        {
            let input = body["input"]
                .as_array_mut()
                .context("Responses request input should be an array")?;
            let observed_order = input[..2]
                .iter()
                .map(|item| item.get("type").and_then(Value::as_str))
                .collect::<Vec<_>>();
            let expected_order = match order {
                ExperimentalResponsesLitePrefixOrder::ToolsThenBase => {
                    vec![Some("additional_tools"), Some("message")]
                }
                ExperimentalResponsesLitePrefixOrder::BaseThenTools => {
                    vec![Some("message"), Some("additional_tools")]
                }
            };
            assert_eq!(observed_order, expected_order);
            let base_index =
                usize::from(order == ExperimentalResponsesLitePrefixOrder::ToolsThenBase);
            assert_eq!(
                input[base_index],
                serde_json::json!({
                    "type": "message",
                    "role": "developer",
                    "content": [{
                        "type": "input_text",
                        "text": "test instructions",
                    }],
                })
            );
        }
        let tools = additional_tools(&body)?.to_vec();
        assert!(!tools.is_empty());
        if let Some(canonical_tools) = canonical_tools.as_ref() {
            assert_eq!(&tools, canonical_tools);
        } else {
            canonical_tools = Some(tools);
        }
    }

    Ok(())
}

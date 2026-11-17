use anyhow::Result;
use core_test_support::responses;
use core_test_support::skip_if_no_network;
use core_test_support::test_codex::test_codex;
use pretty_assertions::assert_eq;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn default_request_preserves_the_stock_catalog_instructions() -> Result<()> {
    skip_if_no_network!(Ok(()));

    const INSTRUCTIONS: &str = "Base instructions.\n\n## Personality\nUse a professional tone.\n\n## Presenting your work and final message\nSummarize the changes.";
    let server = responses::start_mock_server().await;
    let response_mock = responses::mount_sse_once(
        &server,
        responses::sse(vec![
            responses::ev_response_created("resp-1"),
            responses::ev_completed("resp-1"),
        ]),
    )
    .await;
    let test = test_codex()
        .with_model("gpt-6.1-sol")
        .with_model_info_override("gpt-6.1-sol", |model_info| {
            model_info
                .model_messages
                .get_or_insert_default()
                .instructions_template = Some(INSTRUCTIONS.to_string());
        })
        .build_with_auto_env(&server)
        .await?;

    test.submit_turn("hello").await?;
    assert_eq!(
        response_mock.single_request().instructions_text(),
        INSTRUCTIONS
    );
    Ok(())
}

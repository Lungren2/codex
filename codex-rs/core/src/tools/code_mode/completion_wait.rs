//! Coalesce elapsed cell observations without suppressing explicit user preemption.

use std::time::Duration;

use codex_code_mode::RuntimeResponse;
use codex_code_mode::WaitRequest;
use codex_protocol::models::FunctionCallOutputContentItem;
use tokio_util::sync::CancellationToken;

use super::ExecContext;
use super::into_function_call_output_content_items;
use super::telemetry::CodeModeToolCallGuard;
use super::truncate_code_mode_result;

#[derive(Default)]
pub(super) struct ForegroundOutput {
    pub(super) content_items: Vec<FunctionCallOutputContentItem>,
    pub(super) host_duration: Option<Duration>,
}

pub(super) async fn wait_for_completion(
    exec: &ExecContext,
    preempt: Option<CancellationToken>,
    mut response: RuntimeResponse,
    max_output_tokens: Option<usize>,
    telemetry: &mut CodeModeToolCallGuard,
) -> Result<(RuntimeResponse, ForegroundOutput), String> {
    let mut output = ForegroundOutput {
        host_duration: response.code_mode_host_duration(),
        ..Default::default()
    };
    while matches!(response, RuntimeResponse::Yielded { .. })
        && !preempt
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
    {
        let RuntimeResponse::Yielded {
            cell_id,
            content_items,
            ..
        } = response
        else {
            unreachable!("only yielded observations enter the completion wait");
        };
        output
            .content_items
            .extend(into_function_call_output_content_items(content_items));
        output.content_items = truncate_code_mode_result(output.content_items, max_output_tokens);
        let waited = exec
            .session
            .services
            .code_mode_service
            .wait(
                WaitRequest {
                    cell_id,
                    yield_time_ms: 60_000,
                    max_output_tokens,
                },
                preempt.clone(),
            )
            .await?;
        if let Some(duration) = waited.code_mode_host_duration() {
            telemetry.record_code_mode_host_duration(duration);
            output.host_duration = Some(
                output
                    .host_duration
                    .unwrap_or_default()
                    .saturating_add(duration),
            );
        }
        response = waited.into();
    }
    Ok((response, output))
}

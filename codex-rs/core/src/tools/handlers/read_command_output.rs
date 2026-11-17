use std::collections::BTreeMap;

use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolName;
use codex_tools::ToolSpec;
use codex_utils_output_truncation::TruncationPolicy;
use serde::Deserialize;
use uuid::Uuid;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::CoreToolRuntime;
use crate::tools::registry::ToolExecutor;

const TOOL_NAME: &str = "read_command_output";

pub(crate) struct ReadCommandOutputHandler;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadCommandOutputArgs {
    output_id: Uuid,
    #[serde(default)]
    offset: usize,
    query: Option<String>,
}

impl ToolExecutor<ToolInvocation> for ReadCommandOutputHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain(TOOL_NAME)
    }

    fn spec(&self) -> ToolSpec {
        let properties = BTreeMap::from([
            (
                "output_id".to_string(),
                JsonSchema::string(Some("Output ID returned by exec_command diagnostics.".to_string())),
            ),
            (
                "offset".to_string(),
                JsonSchema::integer(Some("UTF-8 byte offset. Use next_offset to continue. Defaults to 0.".to_string())),
            ),
            (
                "query".to_string(),
                JsonSchema::string(Some("Optional case-sensitive literal (up to 256 bytes). Returns text around the first match at or after offset.".to_string())),
            ),
        ]);
        ToolSpec::Function(ResponsesApiTool {
            name: TOOL_NAME.to_string(),
            description: "Read or search a bounded portion of the original command output referred to by a diagnostic view in this chat. The eight most recent retained captures are available during this session.".to_string(),
            strict: false,
            defer_loading: None,
            parameters: JsonSchema::object(
                properties,
                Some(vec!["output_id".to_string()]),
                /*additional_properties*/ Some(false.into()),
            ),
            output_schema: None,
        })
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        true
    }

    fn handle<'a>(&'a self, invocation: ToolInvocation) -> codex_tools::ToolExecutorFuture<'a>
    where
        ToolInvocation: 'a,
    {
        Box::pin(async move {
            let ToolPayload::Function { arguments } = invocation.payload else {
                return Err(FunctionCallError::RespondToModel(format!(
                    "{TOOL_NAME} handler received unsupported payload"
                )));
            };
            let args: ReadCommandOutputArgs = parse_arguments(&arguments)?;
            let policy: TruncationPolicy = invocation
                .step_context
                .settings
                .model_info
                .truncation_policy
                .into();
            let page = invocation
                .session
                .services
                .unified_exec_manager
                .command_output
                .read(
                    args.output_id,
                    args.offset,
                    args.query.as_deref(),
                    policy.byte_budget(),
                )
                .map_err(|message| FunctionCallError::RespondToModel(message.to_string()))?;
            Ok(boxed_tool_output(FunctionToolOutput::from_text(
                page,
                /*success*/ Some(true),
            )))
        })
    }
}

impl CoreToolRuntime for ReadCommandOutputHandler {
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }
}

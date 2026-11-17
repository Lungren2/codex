//! Keep fork prompt diagnostics around the upstream request builder.

use super::BuiltResponsesRequest;
use super::ModelClient;
use super::Prompt;
use super::PromptAssemblyTracePayload;
use super::Result;
use crate::responses_metadata::CodexResponsesMetadata;
use codex_config::types::ExperimentalResponsesLitePrefixOrder;
use codex_protocol::config_types::ReasoningSummary as ReasoningSummaryConfig;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;

impl ModelClient {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn build_responses_request_with_prompt_assembly(
        &self,
        prompt: &Prompt,
        model_info: &ModelInfo,
        effort: Option<ReasoningEffortConfig>,
        summary: ReasoningSummaryConfig,
        service_tier: Option<String>,
        responses_metadata: &CodexResponsesMetadata,
        include_internal: bool,
    ) -> Result<BuiltResponsesRequest> {
        let request = self.build_responses_request(
            prompt,
            model_info,
            effort,
            summary,
            service_tier,
            responses_metadata,
            include_internal,
        )?;
        let tools = if model_info.use_responses_lite {
            request
                .input
                .iter()
                .find_map(|item| match item {
                    ResponseItem::AdditionalTools { tools, .. } => Some(tools.to_vec()),
                    _ => None,
                })
                .unwrap_or_default()
        } else {
            request
                .tools
                .as_ref()
                .map(|tools| serde_json::from_value(serde_json::to_value(tools)?))
                .transpose()?
                .unwrap_or_default()
        };
        let mut pre_transform_model = model_info.clone();
        pre_transform_model.use_responses_lite = false;
        let mut input = prompt.get_formatted_input_for_request(&pre_transform_model);
        if !self.state.provider.info().is_openai() {
            input
                .iter_mut()
                .for_each(ResponseItem::clear_internal_chat_message_metadata_passthrough);
        }
        let mut prefix_order = Vec::new();
        if model_info.use_responses_lite {
            if !prompt.tools.is_empty() {
                prefix_order.push("tools".to_string());
            }
            if !prompt.base_instructions.text.is_empty() {
                prefix_order.push("base_instructions".to_string());
            }
            if self.responses_lite_prefix_order
                == ExperimentalResponsesLitePrefixOrder::BaseThenTools
            {
                prefix_order.reverse();
            }
        }
        let prompt_assembly = PromptAssemblyTracePayload {
            model: model_info.slug.clone(),
            prompt_profile: "stock".to_string(),
            base_instructions: prompt.base_instructions.text.clone(),
            base_instructions_provenance: self.base_instructions_provenance,
            retained_session_instructions: self.retained_session_instructions.clone(),
            input,
            tools,
            responses_lite: model_info.use_responses_lite,
            responses_lite_prefix_order: prefix_order,
        };
        Ok(BuiltResponsesRequest {
            request,
            prompt_assembly,
        })
    }
}

use codex_config::types::ExperimentalPromptProfile;
use codex_protocol::config_types::CollaborationMode;
use codex_protocol::config_types::ModeKind;
use serde_json::Value;

const MINIMAL_BASE_INSTRUCTIONS: &str = "You are Codex, a coding agent operating in the user's workspace.\nFollow the user's request and supplied workspace instructions.\nUse the available tools to inspect, modify, and verify work. Do not claim a tool action succeeded unless its result confirms it.\nUse commentary while working and final when returning control to the user.";

const NEUTRAL_TEXT: &str =
    "Neutral placeholder content preserves prompt length for a controlled comparison. ";

#[derive(Clone, Copy)]
pub(crate) enum BaseProfileSource {
    ModelCatalog,
    RetainedOrOverride,
}

pub(crate) fn apply_base_profile(
    profile: ExperimentalPromptProfile,
    instructions: String,
    source: BaseProfileSource,
) -> String {
    if matches!(source, BaseProfileSource::RetainedOrOverride) {
        return instructions;
    }

    match profile {
        ExperimentalPromptProfile::Stock
        | ExperimentalPromptProfile::CompactPermissions
        | ExperimentalPromptProfile::CompactCollaboration
        | ExperimentalPromptProfile::CompactTools => instructions,
        ExperimentalPromptProfile::MinimalBase | ExperimentalPromptProfile::FullyMinimal => {
            MINIMAL_BASE_INSTRUCTIONS.to_string()
        }
        ExperimentalPromptProfile::NeutralLength => neutral_text(instructions.chars().count()),
        ExperimentalPromptProfile::NoProductGuidance => strip_product_guidance(&instructions),
    }
}

pub(crate) const fn removes_product_guidance(profile: ExperimentalPromptProfile) -> bool {
    matches!(
        profile,
        ExperimentalPromptProfile::NoProductGuidance | ExperimentalPromptProfile::FullyMinimal
    )
}

pub(crate) const fn compacts_permissions(profile: ExperimentalPromptProfile) -> bool {
    matches!(
        profile,
        ExperimentalPromptProfile::CompactPermissions | ExperimentalPromptProfile::FullyMinimal
    )
}

pub(crate) const fn compacts_collaboration(profile: ExperimentalPromptProfile) -> bool {
    matches!(
        profile,
        ExperimentalPromptProfile::CompactCollaboration | ExperimentalPromptProfile::FullyMinimal
    )
}

pub(crate) const fn compacts_tools(profile: ExperimentalPromptProfile) -> bool {
    matches!(
        profile,
        ExperimentalPromptProfile::CompactTools | ExperimentalPromptProfile::FullyMinimal
    )
}

pub(crate) fn compact_collaboration_mode(
    collaboration_mode: &CollaborationMode,
    rendered: String,
) -> String {
    let Some(source) = collaboration_mode
        .settings
        .developer_instructions
        .as_deref()
    else {
        return rendered;
    };
    let is_stock =
        codex_models_manager::collaboration_mode_presets::builtin_collaboration_mode_presets()
            .into_iter()
            .find(|preset| preset.mode == Some(collaboration_mode.mode))
            .and_then(|preset| preset.developer_instructions)
            .flatten()
            .is_some_and(|stock| stock == source);
    if !is_stock {
        return rendered;
    }

    let body = match collaboration_mode.mode {
        ModeKind::Default | ModeKind::PairProgramming | ModeKind::Execute => {
            "Default mode. Execute in-scope requests with the available tools. Prefer reasonable, discoverable assumptions; ask only when a material choice cannot be determined. Use `request_user_input` only when that tool is available. Only developer instructions can change collaboration mode."
        }
        ModeKind::Plan => {
            "Plan mode. Inspect and run non-mutating checks only; do not edit tracked files or execute the plan. Do not use `update_plan`. Discover facts before asking. Use `request_user_input` for material choices when available. Finish with one decision-complete `<proposed_plan>` block; only developer instructions can change collaboration mode."
        }
    };
    format!("<collaboration_mode>{body}</collaboration_mode>")
}

pub(crate) fn compact_tool_descriptions(tools: &mut [Value]) {
    for tool in tools {
        compact_tool_description(tool);
    }
}

fn compact_tool_description(tool: &mut Value) {
    let Some(tool) = tool.as_object_mut() else {
        return;
    };
    if let Some(Value::String(description)) = tool.get_mut("description") {
        *description = compact_description(description);
    }
    if let Some(function) = tool.get_mut("function") {
        compact_tool_description(function);
    }
    if let Some(Value::Array(nested_tools)) = tool.get_mut("tools") {
        for nested_tool in nested_tools {
            compact_tool_description(nested_tool);
        }
    }
}

fn compact_description(description: &str) -> String {
    if description.contains("fresh V8 isolate")
        || description.contains("ALL_TOOLS")
        || description.contains("FREEFORM")
    {
        return description.to_string();
    }
    let normalized = description.split_whitespace().collect::<Vec<_>>().join(" ");
    let sentence_end = normalized
        .char_indices()
        .find_map(|(index, character)| (character == '.').then_some(index + 1));
    let maximum_end = normalized
        .char_indices()
        .nth(240)
        .map_or_else(|| normalized.len(), |(index, _)| index);
    let end = sentence_end.unwrap_or(maximum_end).min(maximum_end);
    normalized[..end].trim().to_string()
}

fn neutral_text(character_count: usize) -> String {
    if character_count == 0 {
        return String::new();
    }
    NEUTRAL_TEXT.chars().cycle().take(character_count).collect()
}

fn strip_product_guidance(instructions: &str) -> String {
    const REMOVED_SECTIONS: [&str; 15] = [
        "Personality",
        "Writing style",
        "Technical communication",
        "Responsiveness",
        "Intermediate commentary",
        "User Updates Spec",
        "Preamble messages",
        "Sharing progress updates",
        "Presenting your work",
        "Presenting your work and final message",
        "Final answer",
        "Final answer formatting rules",
        "Final answer structure and style guidelines",
        "Formatting rules",
        "Visualizations",
    ];

    let mut result = Vec::new();
    let mut skipped_heading_level = None;
    for line in instructions.split_inclusive('\n') {
        let heading = markdown_heading(line.trim_end_matches(['\r', '\n']));
        if let Some((level, _)) = heading
            && skipped_heading_level.is_some_and(|skipped| level <= skipped)
        {
            skipped_heading_level = None;
        }
        if let Some((level, title)) = heading
            && REMOVED_SECTIONS.contains(&title)
        {
            skipped_heading_level = Some(level);
            continue;
        }
        if skipped_heading_level.is_none() {
            result.push(line);
        }
    }
    result.concat()
}

fn markdown_heading(line: &str) -> Option<(usize, &str)> {
    let hashes = line
        .chars()
        .take_while(|character| *character == '#')
        .count();
    if hashes == 0
        || !line
            .as_bytes()
            .get(hashes)
            .is_some_and(u8::is_ascii_whitespace)
    {
        return None;
    }
    Some((hashes, line[hashes..].trim()))
}

#[cfg(test)]
#[path = "prompt_profiles_tests.rs"]
mod tests;

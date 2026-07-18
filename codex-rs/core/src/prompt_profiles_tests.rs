use super::*;
use codex_protocol::config_types::Settings;
use pretty_assertions::assert_eq;
use serde_json::json;

#[test]
fn stock_base_profile_is_byte_identical() {
    let input = "first\r\nsecond\n".to_string();
    assert_eq!(
        apply_base_profile(
            ExperimentalPromptProfile::Stock,
            &ExperimentalPromptProfileOptions::default(),
            input.clone(),
            BaseProfileSource::ModelCatalog,
        ),
        input
    );
}

#[test]
fn config_override_is_preserved_by_every_profile() {
    for profile in [
        ExperimentalPromptProfile::MinimalBase,
        ExperimentalPromptProfile::NeutralLength,
        ExperimentalPromptProfile::NoProductGuidance,
        ExperimentalPromptProfile::FullyMinimal,
        ExperimentalPromptProfile::Custom,
    ] {
        assert_eq!(
            apply_base_profile(
                profile,
                &ExperimentalPromptProfileOptions::default(),
                "user override".to_string(),
                BaseProfileSource::RetainedOrOverride,
            ),
            "user override"
        );
    }
}

#[test]
fn retained_session_base_is_preserved_by_every_profile() {
    for profile in [
        ExperimentalPromptProfile::MinimalBase,
        ExperimentalPromptProfile::NeutralLength,
        ExperimentalPromptProfile::NoProductGuidance,
        ExperimentalPromptProfile::FullyMinimal,
        ExperimentalPromptProfile::Custom,
    ] {
        assert_eq!(
            apply_base_profile(
                profile,
                &ExperimentalPromptProfileOptions::default(),
                "retained instructions".to_string(),
                BaseProfileSource::RetainedOrOverride,
            ),
            "retained instructions"
        );
    }
}

#[test]
fn neutral_profile_preserves_character_length() {
    let source = "alpha βeta\nthird line".to_string();
    let neutral = apply_base_profile(
        ExperimentalPromptProfile::NeutralLength,
        &ExperimentalPromptProfileOptions::default(),
        source.clone(),
        BaseProfileSource::ModelCatalog,
    );
    assert_eq!(neutral.chars().count(), source.chars().count());
    assert_ne!(neutral, source);
}

#[test]
fn neutral_profile_has_a_hard_context_item_cap() {
    let source = "x".repeat(MAX_NEUTRAL_INSTRUCTION_CHARS * 2);
    let neutral = apply_base_profile(
        ExperimentalPromptProfile::NeutralLength,
        &ExperimentalPromptProfileOptions::default(),
        source,
        BaseProfileSource::ModelCatalog,
    );
    assert_eq!(neutral.chars().count(), MAX_NEUTRAL_INSTRUCTION_CHARS);
}

#[test]
fn base_profiles_cover_the_gpt_5_6_model_family_catalog_prompts() {
    let catalog = codex_models_manager::bundled_models_response().expect("bundled model catalog");
    for slug in ["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"] {
        let source = catalog
            .models
            .iter()
            .find(|model| model.slug == slug)
            .unwrap_or_else(|| panic!("missing {slug}"))
            .base_instructions
            .clone();

        assert_eq!(
            apply_base_profile(
                ExperimentalPromptProfile::Stock,
                &ExperimentalPromptProfileOptions::default(),
                source.clone(),
                BaseProfileSource::ModelCatalog,
            ),
            source
        );
        let neutral = apply_base_profile(
            ExperimentalPromptProfile::NeutralLength,
            &ExperimentalPromptProfileOptions::default(),
            source.clone(),
            BaseProfileSource::ModelCatalog,
        );
        assert_eq!(neutral.chars().count(), source.chars().count());
        assert_ne!(neutral, source);

        let minimal = apply_base_profile(
            ExperimentalPromptProfile::MinimalBase,
            &ExperimentalPromptProfileOptions::default(),
            source.clone(),
            BaseProfileSource::ModelCatalog,
        );
        assert_eq!(minimal, MINIMAL_BASE_INSTRUCTIONS);

        let no_product = apply_base_profile(
            ExperimentalPromptProfile::NoProductGuidance,
            &ExperimentalPromptProfileOptions::default(),
            source,
            BaseProfileSource::ModelCatalog,
        );
        assert!(!no_product.contains("# Personality"), "{slug}");
        assert!(!no_product.contains("## Intermediate commentary"), "{slug}");
        assert!(!no_product.contains("## Final answer"), "{slug}");
        assert!(
            no_product.contains("# Rules for getting work done"),
            "{slug}"
        );
        assert!(no_product.contains("# Using skills"), "{slug}");
    }
}

#[test]
fn custom_profile_composes_selected_transformations() {
    let options = ExperimentalPromptProfileOptions {
        base: ExperimentalPromptBase::Stock,
        remove_product_guidance: true,
        compact_permissions: true,
        compact_collaboration: false,
        compact_tools: true,
    };
    let source = "# Personality\nremove\n# Rules\nkeep\n".to_string();
    assert_eq!(
        apply_base_profile(
            ExperimentalPromptProfile::Custom,
            &options,
            source,
            BaseProfileSource::ModelCatalog,
        ),
        "# Rules\nkeep\n"
    );
    assert!(compacts_permissions(
        ExperimentalPromptProfile::Custom,
        &options
    ));
    assert!(!compacts_collaboration(
        ExperimentalPromptProfile::Custom,
        &options
    ));
    assert!(compacts_tools(ExperimentalPromptProfile::Custom, &options));
    assert_eq!(
        profile_name(ExperimentalPromptProfile::Custom, &options),
        "custom:stock+no-product-guidance+compact-permissions+compact-tools"
    );
}

#[test]
fn no_product_guidance_removes_only_named_markdown_sections() {
    let source = "intro\r\n# How you work\r\n## Personality\r\ntone\r\n## Planning\r\nkeep\r\n## Responsiveness\r\nremove\r\n### Preamble messages\r\nremove too\r\n## Task execution\r\nkeep too\r\n## Presenting your work \r\nremove UI\r\n### Final answer structure and style guidelines\r\nremove too\r\n## Shell commands\r\nkeep bytes\r\n";
    assert_eq!(
        strip_product_guidance(source),
        "intro\r\n# How you work\r\n## Planning\r\nkeep\r\n## Task execution\r\nkeep too\r\n## Shell commands\r\nkeep bytes\r\n"
    );
}

#[test]
fn compact_tools_changes_descriptions_but_not_schemas() {
    let schema = json!({"type": "object", "properties": {"path": {"type": "string", "description": "Schema detail stays intact."}}});
    let mut tools = vec![json!({
        "type": "function",
        "name": "inspect",
        "description": "Inspect one item. This tutorial sentence is removed.",
        "parameters": schema
    })];
    compact_tool_descriptions(&mut tools);
    assert_eq!(tools[0]["description"], "Inspect one item.");
    assert_eq!(tools[0]["parameters"], schema);
}

#[test]
fn compact_tools_changes_namespace_and_nested_tool_descriptions_only() {
    let schema = json!({"type": "object", "properties": {"query": {"type": "string", "description": "Nested schema detail stays intact."}}});
    let mut tools = vec![json!({
        "type": "namespace",
        "name": "mcp__demo__",
        "description": "Demo namespace. Additional tutorial text.",
        "tools": [{
            "type": "function",
            "name": "lookup",
            "description": "Look up a value. Additional tutorial text.",
            "parameters": schema
        }]
    })];
    compact_tool_descriptions(&mut tools);
    assert_eq!(tools[0]["description"], "Demo namespace.");
    assert_eq!(tools[0]["tools"][0]["description"], "Look up a value.");
    assert_eq!(tools[0]["tools"][0]["parameters"], schema);
}

#[test]
fn compact_tools_preserves_protocol_bearing_code_mode_description() {
    let description = "Run JavaScript in a fresh V8 isolate. Use ALL_TOOLS exactly as documented.";
    let mut tools = vec![json!({
        "type": "function",
        "name": "exec",
        "description": description,
        "parameters": {"type": "string"}
    })];
    compact_tool_descriptions(&mut tools);
    assert_eq!(tools[0]["description"], description);
}

#[test]
fn compact_collaboration_preserves_mode_limits() {
    let presets =
        codex_models_manager::collaboration_mode_presets::builtin_collaboration_mode_presets();
    let instructions_for = |mode| {
        presets
            .iter()
            .find(|preset| preset.mode == Some(mode))
            .and_then(|preset| preset.developer_instructions.clone())
            .flatten()
            .expect("stock collaboration instructions")
    };
    let default_mode = CollaborationMode {
        mode: ModeKind::Default,
        settings: Settings {
            model: "test".to_string(),
            reasoning_effort: None,
            developer_instructions: Some(instructions_for(ModeKind::Default)),
        },
    };
    let plan_mode = CollaborationMode {
        mode: ModeKind::Plan,
        settings: Settings {
            model: "test".to_string(),
            reasoning_effort: None,
            developer_instructions: Some(instructions_for(ModeKind::Plan)),
        },
    };
    let default = compact_collaboration_mode(&default_mode, "default source".to_string());
    let plan = compact_collaboration_mode(&plan_mode, "plan source".to_string());
    assert!(default.contains("available tools"));
    assert!(default.contains("Only developer instructions"));
    assert!(plan.contains("non-mutating"));
    assert!(plan.contains("Do not use `update_plan`"));
    assert!(plan.contains("<proposed_plan>"));
}

#[test]
fn compact_collaboration_preserves_custom_instructions() {
    let mode = CollaborationMode {
        mode: ModeKind::Default,
        settings: Settings {
            model: "test".to_string(),
            reasoning_effort: None,
            developer_instructions: Some("custom collaboration contract".to_string()),
        },
    };
    assert_eq!(
        compact_collaboration_mode(&mode, "rendered custom contract".to_string()),
        "rendered custom contract"
    );
}

#[test]
fn compact_collaboration_preserves_custom_extension_of_stock_heading() {
    let mode = CollaborationMode {
        mode: ModeKind::Default,
        settings: Settings {
            model: "test".to_string(),
            reasoning_effort: None,
            developer_instructions: Some(
                "# Collaboration Mode: Default\ncustom collaboration contract".to_string(),
            ),
        },
    };
    assert_eq!(
        compact_collaboration_mode(&mode, "rendered custom contract".to_string()),
        "rendered custom contract"
    );
}

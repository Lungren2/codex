# GPT-5.6 prompt-profile artifacts

These 27 artifacts show the effective model-catalog base instructions for each built-in prompt profile and each GPT-5.6 family model (gpt-5.6-sol, gpt-5.6-terra, and gpt-5.6-luna) before user context is included.

All three models currently have byte-identical catalog templates (SHA-256 cbefa6b0bede0e332d957fca70ccacf9f12f4c0ecdf81b819e5cbe1a3b16e265). The distinct files make that equivalence auditable and preserve the complete model/profile matrix.

The prompt profile transformation only applies to model-catalog base instructions. Actual requests additionally receive dynamic, session-specific context before the user message: developer instructions, tools, permission policy, workspace instructions, skills, environment data, and possibly conversation history. The compact-permissions, compact-collaboration, and compact-tools profiles therefore annotate their runtime-only effects rather than inventing a fixed session context.

The custom profile is parameterized rather than a single fixed profile. Its artifact represents the default custom options: base is stock and every boolean switch is false. The configuration can independently select the base (stock, minimal, or neutral-length) and enable remove_product_guidance, compact_permissions, compact_collaboration, and compact_tools.

| Profile | Artifacts |
| --- | --- || `stock` | [gpt-5.6-sol](gpt-5.6-sol--stock.md) · [gpt-5.6-terra](gpt-5.6-terra--stock.md) · [gpt-5.6-luna](gpt-5.6-luna--stock.md) |
| `minimal-base` | [gpt-5.6-sol](gpt-5.6-sol--minimal-base.md) · [gpt-5.6-terra](gpt-5.6-terra--minimal-base.md) · [gpt-5.6-luna](gpt-5.6-luna--minimal-base.md) |
| `neutral-length` | [gpt-5.6-sol](gpt-5.6-sol--neutral-length.md) · [gpt-5.6-terra](gpt-5.6-terra--neutral-length.md) · [gpt-5.6-luna](gpt-5.6-luna--neutral-length.md) |
| `no-product-guidance` | [gpt-5.6-sol](gpt-5.6-sol--no-product-guidance.md) · [gpt-5.6-terra](gpt-5.6-terra--no-product-guidance.md) · [gpt-5.6-luna](gpt-5.6-luna--no-product-guidance.md) |
| `compact-permissions` | [gpt-5.6-sol](gpt-5.6-sol--compact-permissions.md) · [gpt-5.6-terra](gpt-5.6-terra--compact-permissions.md) · [gpt-5.6-luna](gpt-5.6-luna--compact-permissions.md) |
| `compact-collaboration` | [gpt-5.6-sol](gpt-5.6-sol--compact-collaboration.md) · [gpt-5.6-terra](gpt-5.6-terra--compact-collaboration.md) · [gpt-5.6-luna](gpt-5.6-luna--compact-collaboration.md) |
| `compact-tools` | [gpt-5.6-sol](gpt-5.6-sol--compact-tools.md) · [gpt-5.6-terra](gpt-5.6-terra--compact-tools.md) · [gpt-5.6-luna](gpt-5.6-luna--compact-tools.md) |
| `fully-minimal` | [gpt-5.6-sol](gpt-5.6-sol--fully-minimal.md) · [gpt-5.6-terra](gpt-5.6-terra--fully-minimal.md) · [gpt-5.6-luna](gpt-5.6-luna--fully-minimal.md) |
| `custom` | [gpt-5.6-sol](gpt-5.6-sol--custom.md) · [gpt-5.6-terra](gpt-5.6-terra--custom.md) · [gpt-5.6-luna](gpt-5.6-luna--custom.md) |
## Source behavior

- Catalog templates: codex-rs/models-manager/models.json
- Profile resolution and transformations: codex-rs/core/src/prompt_profiles.rs
- Profile definitions: codex-rs/config/src/types.rs

The artifacts are a generated inspection snapshot of those source files, not an independent source of truth.
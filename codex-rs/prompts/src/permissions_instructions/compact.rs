//! Compact view of the same resolved policy facts used by the full renderer.

use super::*;

const MAX_RENDERED_BYTES: usize = 8_000;
const TRUNCATION_MARKER: &str = "\n[compact permissions truncated]\n";

pub(super) fn render(
    context: PermissionsRenderContext<'_>,
    approval: ApprovalPromptContext<'_>,
) -> PermissionsInstructions {
    let sandbox = match context.sandbox_mode {
        SandboxMode::DangerFullAccess => "danger-full-access",
        SandboxMode::WorkspaceWrite => "workspace-write",
        SandboxMode::ReadOnly => "read-only",
    };
    let network = match context.network_access {
        NetworkAccess::Enabled => "enabled",
        NetworkAccess::Restricted => "restricted",
    };
    let mut text =
        format!("Effective permissions: `sandbox_mode={sandbox}`; `network_access={network}`.");
    append_section(&mut text, &compact_approval_text(context, approval));
    if let Some(roots) = writable_roots_text(context.writable_roots, approval.permission_messages) {
        append_section(&mut text, &roots);
    }
    if let Some(denied) = denied_reads_text(
        context.denied_read_paths,
        context.denied_read_globs,
        approval.permission_messages,
    ) {
        append_section(&mut text, &denied);
    }
    text.push('\n');
    let (open, close) = PermissionsInstructions::type_markers();
    let budget = MAX_RENDERED_BYTES.saturating_sub(open.len() + close.len());
    if text.len() > budget {
        let mut end = budget.saturating_sub(TRUNCATION_MARKER.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str(TRUNCATION_MARKER);
    }
    PermissionsInstructions { text }
}

fn compact_approval_text(
    context: PermissionsRenderContext<'_>,
    approval: ApprovalPromptContext<'_>,
) -> String {
    let policy = context.approval_policy;
    let mut text = match policy {
        AskForApproval::Never => "`approval_policy=never`; approval requests are rejected. Do not request escalation.".to_string(),
        AskForApproval::UnlessTrusted => "`approval_policy=unless-trusted`; only the safe read allowlist runs without approval. Request approval for other commands.".to_string(),
        AskForApproval::OnRequest => {
            let mode = match (context.request_permissions_tool_enabled, context.exec_permission_approvals_enabled) {
                (true, true) => "Prefer `with_additional_permissions` or `request_permissions`; use `require_escalated` only when sandboxed permissions cannot satisfy the command.",
                (true, false) => "Prefer `request_permissions`; use `require_escalated` only when sandboxed permissions cannot satisfy the command.",
                (false, true) => "Prefer `with_additional_permissions`; use `require_escalated` only when sandboxed permissions cannot satisfy the command.",
                (false, false) => "Use `require_escalated` with a short `justification` when the command must run outside the sandbox.",
            };
            format!("`approval_policy=on-request`; command segments are evaluated independently. {mode}")
        }
        AskForApproval::Granular(config) => format!(
            "`approval_policy=granular`; sandbox_approval={}; rules={}; skill_approval={}; request_permissions={}; mcp_elicitations={}.",
            config.sandbox_approval, config.rules, config.skill_approval, config.request_permissions, config.mcp_elicitations),
    };
    if let Some(prefixes) = format_allow_prefixes(context.approved_command_prefixes.to_vec()) {
        text.push_str(&format!(" Approved command prefixes: {prefixes}."));
    }
    if approval.reviewer == ApprovalsReviewer::AutoReview && policy != AskForApproval::Never {
        text.push_str(" `approvals_reviewer=auto_review`; escalations are policy-reviewed.");
    }
    text
}

#[cfg(test)]
#[path = "compact_tests.rs"]
mod tests;

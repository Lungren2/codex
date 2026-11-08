use super::*;
use codex_execpolicy::Decision;
use codex_protocol::permissions::FileSystemAccessMode;
use codex_protocol::permissions::FileSystemPath;
use codex_protocol::permissions::FileSystemSandboxEntry;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::permissions::NetworkSandboxPolicy;
use codex_utils_absolute_path::AbsolutePathBuf;
use pretty_assertions::assert_eq;

fn context<'a>(policy: AskForApproval) -> PermissionsRenderContext<'a> {
    PermissionsRenderContext {
        sandbox_mode: SandboxMode::WorkspaceWrite,
        network_access: NetworkAccess::Enabled,
        approval_policy: policy,
        approved_command_prefixes: &[],
        writable_roots: &[],
        denied_read_paths: &[],
        denied_read_globs: &[],
        exec_permission_approvals_enabled: false,
        request_permissions_tool_enabled: false,
    }
}

#[test]
fn compact_permissions_only_name_enabled_request_mechanisms() {
    let approval =
        ApprovalPromptContext::new(ApprovalsReviewer::User, ResolvedModelMessages::bundled());
    let mut facts = context(AskForApproval::OnRequest);
    facts.request_permissions_tool_enabled = true;
    let request_only = render(facts, approval).body().expect_text();
    assert!(request_only.contains("`request_permissions`"));
    assert!(!request_only.contains("`with_additional_permissions`"));
    facts.request_permissions_tool_enabled = false;
    facts.exec_permission_approvals_enabled = true;
    let exec_only = render(facts, approval).body().expect_text();
    assert!(exec_only.contains("`with_additional_permissions`"));
    assert!(!exec_only.contains("`request_permissions`"));
}

#[test]
fn compact_permissions_preserve_policy_and_reviewer_facts() {
    let granular = AskForApproval::Granular(GranularApprovalConfig {
        sandbox_approval: true,
        rules: false,
        skill_approval: true,
        request_permissions: false,
        mcp_elicitations: true,
    });
    for (policy, expected) in [
        (AskForApproval::Never, "approval_policy=never"),
        (
            AskForApproval::UnlessTrusted,
            "approval_policy=unless-trusted",
        ),
        (AskForApproval::OnRequest, "approval_policy=on-request"),
        (granular, "approval_policy=granular"),
    ] {
        for reviewer in [ApprovalsReviewer::User, ApprovalsReviewer::AutoReview] {
            let approval = ApprovalPromptContext::new(reviewer, ResolvedModelMessages::bundled());
            let text = render(context(policy), approval).body().expect_text();
            assert!(text.contains(expected));
            assert_eq!(
                text.contains("approvals_reviewer=auto_review"),
                reviewer == ApprovalsReviewer::AutoReview && policy != AskForApproval::Never
            );
        }
    }
}

#[test]
fn compact_permissions_preserve_roots_denied_reads_and_prefixes() {
    let cwd = std::env::current_dir().expect("current directory");
    let writable_root = AbsolutePathBuf::from_absolute_path(cwd.join("compact-writable")).unwrap();
    let denied_root = AbsolutePathBuf::from_absolute_path(cwd.join("compact-denied")).unwrap();
    let profile = PermissionProfile::from_runtime_permissions(
        &FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry::new(
                FileSystemPath::Path {
                    path: writable_root.clone(),
                },
                FileSystemAccessMode::Write,
            ),
            FileSystemSandboxEntry::new(
                FileSystemPath::Path {
                    path: denied_root.clone(),
                },
                FileSystemAccessMode::Deny,
            ),
        ]),
        NetworkSandboxPolicy::Enabled,
    );
    let mut policy = Policy::empty();
    policy
        .add_prefix_rule(&["git".to_string(), "pull".to_string()], Decision::Allow)
        .unwrap();
    let instructions = PermissionsInstructions::from_permission_profile_compact(
        &profile,
        AskForApproval::OnRequest,
        ApprovalPromptContext::new(
            ApprovalsReviewer::AutoReview,
            ResolvedModelMessages::bundled(),
        ),
        &policy,
        &cwd,
        /*paths*/ None,
        /*exec_permission_approvals_enabled*/ true,
        /*request_permissions_tool_enabled*/ true,
    );
    let text = instructions.body().expect_text();
    for expected in [
        "sandbox_mode=workspace-write",
        "network_access=enabled",
        "approval_policy=on-request",
        "approvals_reviewer=auto_review",
        "with_additional_permissions",
        "request_permissions",
        r#"["git", "pull"]"#,
    ] {
        assert!(text.contains(expected), "missing {expected:?}: {text}");
    }
    assert!(text.contains(writable_root.to_string_lossy().as_ref()));
    assert!(text.contains(denied_root.to_string_lossy().as_ref()));
}

#[test]
fn compact_permissions_have_a_utf8_safe_hard_fragment_cap() {
    let roots = vec!["🦀".repeat(16_000)];
    let mut facts = context(AskForApproval::Never);
    facts.writable_roots = &roots;
    let instructions = render(
        facts,
        ApprovalPromptContext::new(ApprovalsReviewer::User, ResolvedModelMessages::bundled()),
    );
    let rendered = instructions.render().expect_text();
    assert!(rendered.len() <= MAX_RENDERED_BYTES);
    assert!(
        instructions
            .body()
            .expect_text()
            .ends_with(TRUNCATION_MARKER)
    );
}

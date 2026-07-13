use pretty_assertions::assert_eq;

use crate::models::{
    ArtifactStatus, DecisionImportance, DecisionStatus, DiffFile, DiffFileKind, DiffHunk,
    EvidenceRef, HunkCategory, PatchSetIdentity, RepositoryIdentity, ReviewArtifact, ReviewClaim,
    ReviewDecision, ValidationResult, ValidationStatus,
};

use super::{SynthesisContext, SynthesisDocument, build_prompt, output_schema, validate_document};

fn artifact(secret: bool) -> ReviewArtifact {
    ReviewArtifact {
        schema_version: 1,
        artifact_id: "artifact-1".into(),
        thread_id: "thread-1".into(),
        turn_id: "turn-1".into(),
        revision: 1,
        repository: RepositoryIdentity {
            root: "C:\\repo".into(),
            branch: Some("feature".into()),
            base_ref: "main".into(),
            base_sha: "base".into(),
            head_sha: "head".into(),
            worktree_fingerprint: "fingerprint".into(),
        },
        patch_set: PatchSetIdentity {
            diff_sha256: "diff".into(),
            blob_path: "patch.diff".into(),
        },
        title: "Pending".into(),
        summary: String::new(),
        status: ArtifactStatus::Collecting,
        created_at: "2026-07-11T00:00:00Z".into(),
        decisions: Vec::new(),
        validations: vec![ValidationResult {
            id: "test".into(),
            label: "Tests".into(),
            status: ValidationStatus::Passed,
            evidence: Some("ok".into()),
        }],
        hunk_classifications: Vec::new(),
        other_changes: Vec::new(),
        files: vec![DiffFile {
            path: "src/lib.rs".into(),
            old_path: None,
            kind: DiffFileKind::Modify,
            generated: false,
            omitted: false,
            omission_reason: None,
            hunks: vec![DiffHunk {
                id: "hunk-1".into(),
                path: "src/lib.rs".into(),
                header: "@@ -1 +1 @@".into(),
                old_start: 1,
                old_lines: 1,
                new_start: 1,
                new_lines: 1,
                lines: Vec::new(),
                category: HunkCategory::Primary,
                contains_secret: secret,
            }],
        }],
        revision_comparison: Default::default(),
    }
}

fn document(evidence: Vec<EvidenceRef>) -> SynthesisDocument {
    SynthesisDocument {
        title: "Decision review".into(),
        summary: "One decision".into(),
        decisions: vec![ReviewDecision {
            id: "decision-1".into(),
            title: "Change behavior".into(),
            consequence: "Behavior changes".into(),
            summary: "Implements behavior".into(),
            importance: DecisionImportance::Primary,
            risk: None,
            claims: vec![ReviewClaim {
                text: "Behavior changed".into(),
                evidence: evidence.clone(),
                inference: false,
            }],
            evidence,
            validation_ids: vec!["test".into()],
            status: DecisionStatus::Unresolved,
        }],
        other_changes: Vec::new(),
    }
}

#[test]
fn validation_requires_known_and_complete_hunk_coverage() {
    let artifact = artifact(false);
    let reference = EvidenceRef {
        path: "src/lib.rs".into(),
        hunk_id: "hunk-1".into(),
    };
    assert!(validate_document(&artifact, &document(vec![reference])).is_ok());

    let missing = document(Vec::new());
    assert!(validate_document(&artifact, &missing).is_err());
}

#[test]
fn prompt_redacts_secret_hunk_bodies_and_honors_cap() {
    let artifact = artifact(true);
    let context = SynthesisContext {
        patch_workspace: "C:\\patch".into(),
        original_request: "do work".into(),
        final_response: "done".into(),
        repository_instructions: "rules".into(),
    };
    let prompt = build_prompt(&artifact, &context, 8_000);
    assert!(prompt.contains("[REDACTED: LOCAL SECRET PATTERN DETECTED]"));
    assert!(prompt.contains("hunk-1"));
    assert!(prompt.len() <= 8_000);
}

#[test]
fn output_schema_is_strict_at_the_root() {
    let schema = output_schema();
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["required"][3], "otherChanges");
}

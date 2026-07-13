use super::*;
use crate::models::*;
use pretty_assertions::assert_eq;

fn artifact() -> ReviewArtifact {
    ReviewArtifact {
        schema_version: 1,
        artifact_id: "artifact-1".into(),
        thread_id: "thread-1".into(),
        turn_id: "turn-1".into(),
        revision: 1,
        repository: RepositoryIdentity {
            root: "C:/repo".into(),
            branch: Some("topic".into()),
            base_ref: "origin/main".into(),
            base_sha: "a".into(),
            head_sha: "b".into(),
            worktree_fingerprint: "f".into(),
        },
        patch_set: PatchSetIdentity {
            diff_sha256: "d".into(),
            blob_path: "patch".into(),
        },
        title: "Review topic".into(),
        summary: "Captured".into(),
        status: ArtifactStatus::Collecting,
        created_at: "2026-07-11T00:00:00Z".into(),
        decisions: vec![],
        validations: vec![],
        hunk_classifications: vec![],
        other_changes: vec![],
        files: vec![],
        revision_comparison: Default::default(),
    }
}

#[test]
fn persists_capture_once_and_lists_it() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("review.db")).unwrap();
    let expected = artifact();
    assert_eq!(
        store.persist_capture("{}", &expected).unwrap(),
        "artifact-1"
    );
    assert_eq!(
        store.persist_capture("{}", &expected).unwrap(),
        "artifact-1"
    );
    assert_eq!(store.get_artifact("artifact-1").unwrap(), Some(expected));
    assert_eq!(store.list_artifacts().unwrap().len(), 1);
}

#[test]
fn stores_repository_base_override() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("review.db")).unwrap();
    store
        .set_repository_base_override("C:/repo", "upstream/main")
        .unwrap();
    assert_eq!(
        store
            .repository_base_override("C:/repo")
            .unwrap()
            .as_deref(),
        Some("upstream/main")
    );
}

#[test]
fn rejects_feedback_for_a_different_patch() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("review.db")).unwrap();
    store.persist_capture("{}", &artifact()).unwrap();
    let feedback = ReviewFeedback {
        artifact_id: "artifact-1".into(),
        revision: 1,
        diff_sha256: "wrong".into(),
        scope: FeedbackScope::Artifact,
        comment: "change it".into(),
        allow_stale: false,
        stale: false,
    };
    assert!(
        store
            .queue_feedback(&feedback)
            .unwrap_err()
            .to_string()
            .contains("stale")
    );
}

#[test]
fn revision_comparison_distinguishes_unchanged_new_and_invalidated_decisions() {
    let mut previous = artifact();
    previous.revision = 1;
    previous.decisions = vec![decision("same", "hunk-1"), decision("removed", "hunk-2")];
    let mut current = artifact();
    current.revision = 2;
    current.decisions = vec![decision("same", "hunk-1"), decision("new", "hunk-3")];

    let comparison = compare_revisions(Some(&previous), &current);
    assert_eq!(comparison.previous_revision, Some(1));
    assert_eq!(comparison.unchanged_decision_ids, vec!["same"]);
    assert_eq!(comparison.new_decision_ids, vec!["new"]);
    assert_eq!(comparison.invalidated_decision_ids, vec!["removed"]);
}

fn decision(id: &str, hunk_id: &str) -> ReviewDecision {
    ReviewDecision {
        id: id.to_string(),
        title: id.to_string(),
        consequence: "consequence".into(),
        summary: "summary".into(),
        importance: DecisionImportance::Primary,
        risk: None,
        claims: Vec::new(),
        evidence: vec![EvidenceRef {
            path: "src/lib.rs".into(),
            hunk_id: hunk_id.to_string(),
        }],
        validation_ids: Vec::new(),
        status: DecisionStatus::Unresolved,
    }
}

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ArtifactStatus {
    Collecting,
    Synthesizing,
    Ready,
    Partial,
    Stale,
    Superseded,
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum HunkCategory {
    Primary,
    Supporting,
    Generated,
    Incidental,
    RiskSensitive,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceRef {
    pub path: String,
    pub hunk_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewClaim {
    pub text: String,
    pub evidence: Vec<EvidenceRef>,
    pub inference: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewDecision {
    pub id: String,
    pub title: String,
    pub consequence: String,
    pub summary: String,
    pub importance: DecisionImportance,
    pub risk: Option<String>,
    pub claims: Vec<ReviewClaim>,
    pub evidence: Vec<EvidenceRef>,
    pub validation_ids: Vec<String>,
    pub status: DecisionStatus,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DecisionImportance {
    Primary,
    Supporting,
    RiskSensitive,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DecisionStatus {
    Unresolved,
    Accepted,
    ChangesRequested,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ValidationResult {
    pub id: String,
    pub label: String,
    pub status: ValidationStatus,
    pub evidence: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ValidationStatus {
    Passed,
    Failed,
    NotRun,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HunkClassification {
    pub hunk_id: String,
    pub category: HunkCategory,
    pub decision_ids: Vec<String>,
    pub contains_secret: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub content: String,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DiffLineKind {
    Context,
    Addition,
    Deletion,
    Meta,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiffHunk {
    pub id: String,
    pub path: String,
    pub header: String,
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<DiffLine>,
    pub category: HunkCategory,
    pub contains_secret: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiffFile {
    pub path: String,
    pub old_path: Option<String>,
    pub kind: DiffFileKind,
    pub generated: bool,
    pub omitted: bool,
    pub omission_reason: Option<String>,
    pub hunks: Vec<DiffHunk>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DiffFileKind {
    Add,
    Delete,
    Modify,
    Rename,
    Binary,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryIdentity {
    pub root: String,
    pub branch: Option<String>,
    pub base_ref: String,
    pub base_sha: String,
    pub head_sha: String,
    pub worktree_fingerprint: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PatchSetIdentity {
    pub diff_sha256: String,
    pub blob_path: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RevisionComparison {
    pub previous_revision: Option<u32>,
    pub unchanged_decision_ids: Vec<String>,
    pub addressed_decision_ids: Vec<String>,
    pub new_decision_ids: Vec<String>,
    pub invalidated_decision_ids: Vec<String>,
}

impl Default for RevisionComparison {
    fn default() -> Self {
        Self {
            previous_revision: None,
            unchanged_decision_ids: Vec::new(),
            addressed_decision_ids: Vec::new(),
            new_decision_ids: Vec::new(),
            invalidated_decision_ids: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewArtifact {
    pub schema_version: u8,
    pub artifact_id: String,
    pub thread_id: String,
    pub turn_id: String,
    pub revision: u32,
    pub repository: RepositoryIdentity,
    pub patch_set: PatchSetIdentity,
    pub title: String,
    pub summary: String,
    pub status: ArtifactStatus,
    pub created_at: String,
    pub decisions: Vec<ReviewDecision>,
    pub validations: Vec<ValidationResult>,
    pub hunk_classifications: Vec<HunkClassification>,
    pub other_changes: Vec<EvidenceRef>,
    pub files: Vec<DiffFile>,
    #[serde(default)]
    pub revision_comparison: RevisionComparison,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum FeedbackScope {
    Artifact,
    File {
        path: String,
    },
    Decision {
        decision_id: String,
    },
    Hunk {
        path: String,
        hunk_id: String,
    },
    Lines {
        path: String,
        hunk_id: String,
        side: DiffSide,
        start_line: u32,
        end_line: u32,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DiffSide {
    Old,
    New,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFeedback {
    pub artifact_id: String,
    pub revision: u32,
    pub diff_sha256: String,
    pub scope: FeedbackScope,
    pub comment: String,
    #[serde(default)]
    pub allow_stale: bool,
    #[serde(default)]
    pub stale: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackRecord {
    pub id: String,
    pub status: String,
    pub feedback: ReviewFeedback,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactListItem {
    pub artifact_id: String,
    pub thread_id: String,
    pub turn_id: String,
    pub revision: u32,
    pub title: String,
    pub summary: String,
    pub status: ArtifactStatus,
    pub created_at: String,
    pub repository_root: String,
    pub branch: Option<String>,
    pub decision_count: usize,
    pub unresolved_count: usize,
}

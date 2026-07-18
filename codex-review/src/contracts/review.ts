export type ArtifactStatus =
  | "collecting"
  | "synthesizing"
  | "ready"
  | "partial"
  | "stale"
  | "superseded"
  | "unavailable";

export type HunkCategory =
  | "primary"
  | "supporting"
  | "generated"
  | "incidental"
  | "riskSensitive";

export type EvidenceRef = {
  path: string;
  hunkId: string;
};

export type ReviewClaim = {
  text: string;
  evidence: EvidenceRef[];
  inference: boolean;
};

export type ReviewDecision = {
  id: string;
  title: string;
  consequence: string;
  summary: string;
  importance: "primary" | "supporting" | "riskSensitive";
  risk: string | null;
  claims: ReviewClaim[];
  evidence: EvidenceRef[];
  validationIds: string[];
  status: "unresolved" | "accepted" | "changesRequested";
};

export type ValidationResult = {
  id: string;
  label: string;
  status: "passed" | "failed" | "notRun";
  evidence: string | null;
};

export type HunkClassification = {
  hunkId: string;
  category: HunkCategory;
  decisionIds: string[];
  containsSecret: boolean;
};

export type DiffLine = {
  kind: "context" | "addition" | "deletion" | "meta";
  content: string;
  oldLine: number | null;
  newLine: number | null;
};

export type DiffHunk = {
  id: string;
  path: string;
  header: string;
  oldStart: number;
  oldLines: number;
  newStart: number;
  newLines: number;
  lines: DiffLine[];
  category: HunkCategory;
  containsSecret: boolean;
};

export type DiffFile = {
  path: string;
  oldPath: string | null;
  kind: "add" | "delete" | "modify" | "rename" | "binary";
  generated: boolean;
  omitted: boolean;
  omissionReason: string | null;
  hunks: DiffHunk[];
};

export type ReviewArtifact = {
  schemaVersion: 1;
  artifactId: string;
  threadId: string;
  turnId: string;
  revision: number;
  repository: {
    root: string;
    branch: string | null;
    baseRef: string;
    baseSha: string;
    headSha: string;
    worktreeFingerprint: string;
  };
  patchSet: {
    diffSha256: string;
    blobPath: string;
  };
  title: string;
  summary: string;
  status: ArtifactStatus;
  createdAt: string;
  decisions: ReviewDecision[];
  validations: ValidationResult[];
  hunkClassifications: HunkClassification[];
  otherChanges: EvidenceRef[];
  files: DiffFile[];
  revisionComparison: {
    previousRevision: number | null;
    unchangedDecisionIds: string[];
    addressedDecisionIds: string[];
    newDecisionIds: string[];
    invalidatedDecisionIds: string[];
  };
};

export type FeedbackScope =
  | { type: "artifact" }
  | { type: "file"; path: string }
  | { type: "decision"; decisionId: string }
  | { type: "hunk"; path: string; hunkId: string }
  | {
      type: "lines";
      path: string;
      hunkId: string;
      side: "old" | "new";
      startLine: number;
      endLine: number;
    };

export type ReviewFeedback = {
  artifactId: string;
  revision: number;
  diffSha256: string;
  scope: FeedbackScope;
  comment: string;
  allowStale: boolean;
  stale: boolean;
};

export type FeedbackRecord = {
  id: string;
  status: "draft" | "queued" | "running" | "answeredNoChange" | "addressed" | "failedRetryable" | "orphaned";
  feedback: ReviewFeedback;
  createdAt: string;
  updatedAt: string;
};

export type ArtifactListItem = Pick<
  ReviewArtifact,
  "artifactId" | "threadId" | "turnId" | "revision" | "title" | "summary" | "status" | "createdAt"
> & {
  repositoryRoot: string;
  branch: string | null;
  decisionCount: number;
  unresolvedCount: number;
};

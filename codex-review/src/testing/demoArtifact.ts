import type { ReviewArtifact } from "../contracts/review";

const hunkId = "hunk-feedback-envelope";

export const demoArtifact: ReviewArtifact = {
  schemaVersion: 1,
  artifactId: "review-demo",
  threadId: "019f4f77-demo",
  turnId: "turn-demo",
  revision: 2,
  repository: {
    root: "D:\\dev\\codex-review",
    branch: "main",
    baseRef: "origin/main",
    baseSha: "9b148f2",
    headSha: "c41a670",
    worktreeFingerprint: "c41a670-eef72cb",
  },
  patchSet: {
    diffSha256: "eef72cbda92b5cf8",
    blobPath: "patches/eef72cbda92b5cf8.diff",
  },
  title: "Scope feedback to immutable review evidence",
  summary: "Feedback now carries the exact artifact revision and hunk identity, preventing comments from silently drifting onto newer code.",
  status: "ready",
  createdAt: "2026-07-11T09:42:00Z",
  decisions: [
    {
      id: "scoped-feedback",
      title: "Anchor feedback to a frozen patch",
      consequence: "Comments cannot be applied to a different revision without an explicit re-anchor.",
      summary: "The request envelope now includes the diff fingerprint, selected hunk, and line side.",
      importance: "riskSensitive",
      risk: "Cross-process task ownership remains unproven, so feedback is queued for the desktop task rather than auto-resumed.",
      claims: [
        {
          text: "Each request carries the immutable diff fingerprint.",
          evidence: [{ path: "src/contracts/review.ts", hunkId }],
          inference: false,
        },
      ],
      evidence: [{ path: "src/contracts/review.ts", hunkId }],
      validationIds: ["typecheck"],
      status: "unresolved",
    },
    {
      id: "fallback-flow",
      title: "Keep desktop task ownership safe",
      consequence: "The companion opens the original task and copies a scoped prompt instead of racing its rollout.",
      summary: "Automatic resume remains disabled until a shared owner or official desktop connection exists.",
      importance: "supporting",
      risk: null,
      claims: [],
      evidence: [],
      validationIds: [],
      status: "accepted",
    },
  ],
  validations: [
    { id: "typecheck", label: "TypeScript contract check", status: "passed", evidence: "pnpm build" },
    { id: "desktop", label: "Desktop feedback handoff", status: "notRun", evidence: null },
  ],
  hunkClassifications: [
    { hunkId, category: "riskSensitive", decisionIds: ["scoped-feedback"], containsSecret: false },
  ],
  otherChanges: [],
  files: [
    {
      path: "src/contracts/review.ts",
      oldPath: null,
      kind: "modify",
      generated: false,
      omitted: false,
      omissionReason: null,
      hunks: [
        {
          id: hunkId,
          path: "src/contracts/review.ts",
          header: "@@ -88,6 +88,14 @@ export type ReviewFeedback = {",
          oldStart: 88,
          oldLines: 6,
          newStart: 88,
          newLines: 14,
          category: "riskSensitive",
          containsSecret: false,
          lines: [
            { kind: "context", content: "  artifactId: string;", oldLine: 88, newLine: 88 },
            { kind: "addition", content: "  revision: number;", oldLine: null, newLine: 89 },
            { kind: "addition", content: "  diffSha256: string;", oldLine: null, newLine: 90 },
            { kind: "addition", content: "  scope: FeedbackScope;", oldLine: null, newLine: 91 },
            { kind: "context", content: "  comment: string;", oldLine: 89, newLine: 92 },
          ],
        },
      ],
    },
  ],
  revisionComparison: {
    previousRevision: 1,
    unchangedDecisionIds: ["fallback-flow"],
    addressedDecisionIds: ["scoped-feedback"],
    newDecisionIds: [],
    invalidatedDecisionIds: [],
  },
};

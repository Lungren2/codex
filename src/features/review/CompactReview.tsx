import type { Dispatch, SetStateAction } from "react";
import type {
  DiffHunk,
  FeedbackRecord,
  FeedbackScope,
  ReviewArtifact,
  ReviewDecision,
} from "../../contracts/review";
import { DiffView } from "./components/DiffView";

type Props = {
  artifact: ReviewArtifact;
  selectedDecision: ReviewDecision | null;
  hunks: DiffHunk[];
  scope: FeedbackScope;
  setScope: Dispatch<SetStateAction<FeedbackScope>>;
  comment: string;
  setComment: Dispatch<SetStateAction<string>>;
  feedbackState: "draft" | "queued" | "failed";
  feedbackError: string | null;
  allowStale: boolean;
  setAllowStale: Dispatch<SetStateAction<boolean>>;
  handoffPrompt: string | null;
  handoffThreadId: string | null;
  feedbackHistory: FeedbackRecord[];
  onSubmit: () => void;
  onRecapture: () => void;
  onSelectHunk: (hunk: DiffHunk) => void;
  onActiveHunk: (hunk: DiffHunk) => void;
  onCommentChange: (value: string) => void;
  onLineSelection: (hunk: DiffHunk, side: "old" | "new", startLine: number, endLine: number) => void;
};

export function CompactReview({
  artifact,
  selectedDecision,
  hunks,
  scope,
  setScope,
  comment,
  feedbackState,
  feedbackError,
  allowStale,
  setAllowStale,
  handoffPrompt,
  handoffThreadId,
  feedbackHistory,
  onSubmit,
  onRecapture,
  onSelectHunk,
  onActiveHunk,
  onCommentChange,
  onLineSelection,
}: Props) {
  const activePath = scope.type === "file" || scope.type === "hunk" || scope.type === "lines"
    ? scope.path
    : selectedDecision?.evidence[0]?.path ?? artifact.files[0]?.path ?? "";
  const changedLines = artifact.files.reduce(
    (total, file) => total + file.hunks.reduce((count, hunk) => count + hunk.lines.filter((line) => line.kind === "addition" || line.kind === "deletion").length, 0),
    0,
  );
  const contextualFeedback = feedbackHistory.filter((record) => feedbackMatches(record.feedback.scope, scope));
  const blocked = artifact.status === "stale" || artifact.status === "unavailable";

  return (
    <main className="compact-review">
      <header className="compact-bar">
        <strong>{repositoryName(artifact.repository.root)}</strong>
        <span>{artifact.repository.branch ?? "detached"}</span>
        <span>← {artifact.repository.baseRef}</span>
        <span>{artifact.files.length} files</span>
        <span>±{changedLines}</span>
        <span className={`compact-state compact-state--${artifact.status}`}>{artifact.status === "ready" ? "current" : artifact.status}</span>
      </header>

      <aside className="compact-rail">
        <nav className="file-list" aria-label="Changed files">
          {artifact.files.map((file) => {
            const additions = file.hunks.flatMap((hunk) => hunk.lines).filter((line) => line.kind === "addition").length;
            const deletions = file.hunks.flatMap((hunk) => hunk.lines).filter((line) => line.kind === "deletion").length;
            return (
              <button
                type="button"
                key={file.path}
                className={file.path === activePath ? "is-active" : ""}
                onClick={() => {
                  const hunk = file.hunks[0];
                  if (!hunk) return;
                  onSelectHunk(hunk);
                  document.getElementById(hunk.id)?.scrollIntoView({ block: "start" });
                }}
              >
                <span>{file.path}</span>
                <small><i>-{deletions}</i> <b>+{additions}</b></small>
              </button>
            );
          })}
        </nav>

        <section className="file-context" aria-label="Active file explanation">
          <code>{activePath}</code>
          <p>{selectedDecision?.summary ?? artifact.summary}</p>
          {selectedDecision?.risk && <p className="context-risk">Risk: {selectedDecision.risk}</p>}
          <p className="context-validation">
            {validationText(artifact, selectedDecision)}
          </p>
        </section>

        <form className="compact-comment" onSubmit={(event) => { event.preventDefault(); onSubmit(); }}>
          <div className="scope-line">
            <span>{scopeLabel(scope)}</span>
            <select
              aria-label="Feedback scope"
              value={scope.type === "lines" || scope.type === "hunk" ? scope.type : scope.type === "file" ? "file" : "artifact"}
              onChange={(event) => {
                if (event.target.value === "artifact") setScope({ type: "artifact" });
                if (event.target.value === "file" && activePath) setScope({ type: "file", path: activePath });
              }}
            >
              {(scope.type === "lines" || scope.type === "hunk") && <option value={scope.type}>{scope.type === "lines" ? "Selected lines" : "Current hunk"}</option>}
              <option value="file">Current file</option>
              <option value="artifact">Entire diff</option>
            </select>
          </div>
          <textarea
            aria-label="Request change"
            value={comment}
            onChange={(event) => onCommentChange(event.target.value)}
            placeholder={`Request change to ${activePath || "this diff"}`}
            rows={4}
          />
          {feedbackError?.includes("Confirm a stale") && (
            <label className="compact-stale"><input type="checkbox" checked={allowStale} onChange={(event) => setAllowStale(event.target.checked)} /> Send against older revision</label>
          )}
          {blocked ? (
            <button type="button" onClick={onRecapture}>Refresh frozen diff</button>
          ) : (
            <button type="submit" disabled={!comment.trim() || feedbackState === "queued"}>
              {feedbackState === "queued" ? "Copied to Codex" : feedbackState === "failed" ? "Retry" : "Send"}
            </button>
          )}
          <output className={`compact-result compact-result--${feedbackState}`}>
            {feedbackState === "queued" ? `Saved · task ${handoffThreadId?.slice(0, 8) ?? "opened"}` : feedbackError ?? ""}
          </output>
          {handoffPrompt && <details><summary>Copied request</summary><textarea readOnly value={handoffPrompt} rows={5} /></details>}
          {contextualFeedback.slice(0, 2).map((record) => <p className="compact-history" key={record.id}><span>{record.status}</span>{record.feedback.comment}</p>)}
        </form>
      </aside>

      <section className="compact-diff" aria-label="Complete diff">
        <DiffView
          hunks={hunks}
          mode="split"
          activeScope={scope}
          onSelectHunk={onSelectHunk}
          onActiveHunk={onActiveHunk}
          onSelectLines={onLineSelection}
        />
      </section>
    </main>
  );
}

function repositoryName(root: string) {
  return root.replace(/[\\/]+$/, "").split(/[\\/]/).at(-1) ?? root;
}

function scopeLabel(scope: FeedbackScope) {
  if (scope.type === "lines") return `${scope.path}:${scope.startLine}-${scope.endLine}`;
  if (scope.type === "hunk") return `${scope.path} · hunk`;
  if (scope.type === "file") return scope.path;
  return "Entire diff";
}

function validationText(artifact: ReviewArtifact, decision: ReviewDecision | null) {
  const validations = decision
    ? artifact.validations.filter((validation) => decision.validationIds.includes(validation.id))
    : artifact.validations;
  return validations.map((validation) => `${validation.label}: ${validation.status}`).join(" · ") || "No validation captured";
}

function feedbackMatches(record: FeedbackScope, current: FeedbackScope) {
  if (record.type === "artifact" || current.type === "artifact") return true;
  if (record.type === "file" && current.type === "file") return record.path === current.path;
  if ((record.type === "hunk" || record.type === "lines") && (current.type === "hunk" || current.type === "lines")) {
    return record.path === current.path && record.hunkId === current.hunkId;
  }
  return record.type === "decision" && current.type === "decision" && record.decisionId === current.decisionId;
}

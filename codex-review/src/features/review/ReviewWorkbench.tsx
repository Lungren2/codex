import { useCallback, useEffect, useMemo, useState } from "react";
import type { DiffHunk, FeedbackRecord, FeedbackScope, ReviewArtifact, ReviewFeedback } from "../../contracts/review";
import {
  captureRepository,
  getArtifact,
  listArtifacts,
  listFeedback,
  prepareFeedbackHandoff,
  queueFeedback,
  setRepositoryBase,
} from "../../lib/backend";
import { CompactReview } from "./CompactReview";

export function ReviewWorkbench() {
  const [artifact, setArtifact] = useState<ReviewArtifact | null>(null);
  const [scope, setScope] = useState<FeedbackScope>({ type: "artifact" });
  const [comment, setComment] = useState("");
  const [feedbackState, setFeedbackState] = useState<"draft" | "queued" | "failed">("draft");
  const [feedbackError, setFeedbackError] = useState<string | null>(null);
  const [allowStale, setAllowStale] = useState(false);
  const [handoffPrompt, setHandoffPrompt] = useState<string | null>(null);
  const [handoffThreadId, setHandoffThreadId] = useState<string | null>(null);
  const [feedbackHistory, setFeedbackHistory] = useState<FeedbackRecord[]>([]);
  const [loadingMessage, setLoadingMessage] = useState("Loading reviews…");
  const [manualRoot, setManualRoot] = useState("");
  const [manualBase, setManualBase] = useState("");
  const [manualError, setManualError] = useState<string | null>(null);

  useEffect(() => {
    void listArtifacts()
      .then(async ([first]) => {
        if (!first) {
          setLoadingMessage("No frozen diffs yet");
          return;
        }
        const loaded = await getArtifact(first.artifactId);
        setArtifact(loaded);
        setFeedbackHistory(await listFeedback(loaded.artifactId));
        const firstHunk = loaded.files.flatMap((file) => file.hunks)[0];
        if (firstHunk) setScope({ type: "hunk", path: firstHunk.path, hunkId: firstHunk.id });
      })
      .catch((error: unknown) => setLoadingMessage(String(error)));
  }, []);

  const selectedDecision = useMemo(() => {
    if (!artifact) return null;
    if (scope.type === "decision") return artifact.decisions.find((decision) => decision.id === scope.decisionId) ?? null;
    if (scope.type === "file") return artifact.decisions.find((decision) => decision.evidence.some((evidence) => evidence.path === scope.path)) ?? null;
    if (scope.type === "hunk" || scope.type === "lines") {
      const ids = artifact.hunkClassifications.find((item) => item.hunkId === scope.hunkId)?.decisionIds ?? [];
      return artifact.decisions.find((decision) => ids.includes(decision.id)) ?? null;
    }
    return artifact.decisions[0] ?? null;
  }, [artifact, scope]);

  const followVisibleHunk = useCallback((hunk: DiffHunk) => {
    setScope((current) => current.type === "lines" ? current : { type: "hunk", path: hunk.path, hunkId: hunk.id });
  }, []);

  async function submitFeedback() {
    if (!artifact || !comment.trim()) return;
    const feedback: ReviewFeedback = {
      artifactId: artifact.artifactId,
      revision: artifact.revision,
      diffSha256: artifact.patchSet.diffSha256,
      scope,
      comment: comment.trim(),
      allowStale,
      stale: false,
    };
    try {
      const id = await queueFeedback(feedback);
      const handoff = await prepareFeedbackHandoff(id);
      await navigator.clipboard.writeText(handoff.prompt);
      setHandoffPrompt(handoff.prompt);
      setHandoffThreadId(handoff.threadId);
      setFeedbackHistory(await listFeedback(artifact.artifactId));
      setFeedbackError(null);
      setFeedbackState("queued");
    } catch (error: unknown) {
      setFeedbackError(String(error));
      setFeedbackState("failed");
    }
  }

  async function recapture() {
    if (!artifact) return;
    const id = await captureRepository(artifact.repository.root);
    const refreshed = await getArtifact(id);
    setArtifact(refreshed);
    setScope({ type: "artifact" });
    setFeedbackState("draft");
  }

  async function createManualReview() {
    if (!manualRoot.trim()) return;
    setManualError(null);
    try {
      if (manualBase.trim()) await setRepositoryBase(manualRoot.trim(), manualBase.trim());
      const id = await captureRepository(manualRoot.trim());
      const created = await getArtifact(id);
      setArtifact(created);
      setFeedbackHistory([]);
      const firstHunk = created.files.flatMap((file) => file.hunks)[0];
      setScope(firstHunk ? { type: "hunk", path: firstHunk.path, hunkId: firstHunk.id } : { type: "artifact" });
    } catch (error: unknown) {
      setManualError(String(error));
    }
  }

  if (!artifact) {
    return (
      <main className="compact-empty">
        <span>{loadingMessage}</span>
        <form onSubmit={(event) => { event.preventDefault(); void createManualReview(); }}>
          <input aria-label="Repository path" value={manualRoot} onChange={(event) => setManualRoot(event.target.value)} placeholder="D:\dev\repository" />
          <input aria-label="Base ref" value={manualBase} onChange={(event) => setManualBase(event.target.value)} placeholder="base ref (optional)" />
          <button type="submit" disabled={!manualRoot.trim()}>Open diff</button>
        </form>
        {manualError && <output>{manualError}</output>}
      </main>
    );
  }

  return (
    <CompactReview
      artifact={artifact}
      selectedDecision={selectedDecision}
      hunks={artifact.files.flatMap((file) => file.hunks)}
      scope={scope}
      setScope={setScope}
      comment={comment}
      setComment={setComment}
      feedbackState={feedbackState}
      feedbackError={feedbackError}
      allowStale={allowStale}
      setAllowStale={setAllowStale}
      handoffPrompt={handoffPrompt}
      handoffThreadId={handoffThreadId}
      feedbackHistory={feedbackHistory}
      onSubmit={() => void submitFeedback()}
      onRecapture={() => void recapture()}
      onSelectHunk={(hunk) => setScope({ type: "hunk", path: hunk.path, hunkId: hunk.id })}
      onActiveHunk={followVisibleHunk}
      onCommentChange={(value) => {
        setComment(value);
        setFeedbackState("draft");
        setFeedbackError(null);
        setHandoffPrompt(null);
      }}
      onLineSelection={(hunk, side, startLine, endLine) => {
        setScope({ type: "lines", path: hunk.path, hunkId: hunk.id, side, startLine, endLine });
        setFeedbackState("draft");
      }}
    />
  );
}

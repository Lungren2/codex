import { invoke } from "@tauri-apps/api/core";
import type { ArtifactListItem, FeedbackRecord, ReviewArtifact, ReviewFeedback } from "../contracts/review";
import { demoArtifact } from "../testing/demoArtifact";

const inTauri = () => "__TAURI_INTERNALS__" in window;
const demoMode = () => new URLSearchParams(window.location.search).has("demo");

export async function listArtifacts(): Promise<ArtifactListItem[]> {
  if (!inTauri() || demoMode()) {
    return [toListItem(demoArtifact)];
  }
  return invoke<ArtifactListItem[]>("list_artifacts");
}

export async function getArtifact(artifactId: string): Promise<ReviewArtifact> {
  if (!inTauri() || demoMode()) {
    return demoArtifact;
  }
  return invoke<ReviewArtifact>("get_artifact", { artifactId });
}

export async function queueFeedback(feedback: ReviewFeedback): Promise<string> {
  if (!inTauri() || demoMode()) {
    return "preview-feedback";
  }
  return invoke<string>("queue_feedback", { feedback });
}

export async function listFeedback(artifactId: string): Promise<FeedbackRecord[]> {
  if (!inTauri() || demoMode()) return [];
  return invoke<FeedbackRecord[]>("list_feedback", { artifactId });
}

export type FeedbackHandoff = { threadId: string; prompt: string };

export async function prepareFeedbackHandoff(feedbackId: string): Promise<FeedbackHandoff> {
  if (!inTauri() || demoMode()) {
    return {
      threadId: demoArtifact.threadId,
      prompt: "Please address this scoped review feedback in the originating Codex task.",
    };
  }
  return invoke<FeedbackHandoff>("prepare_feedback_handoff", { feedbackId });
}

export async function captureRepository(repositoryRoot: string): Promise<string> {
  if (!inTauri() || demoMode()) {
    return demoArtifact.artifactId;
  }
  return invoke<string>("capture_repository", { repositoryRoot });
}

export async function setRepositoryBase(repositoryRoot: string, baseRef: string): Promise<void> {
  if (!inTauri() || demoMode()) return;
  return invoke<void>("set_repository_base", { repositoryRoot, baseRef });
}

function toListItem(artifact: ReviewArtifact): ArtifactListItem {
  return {
    artifactId: artifact.artifactId,
    threadId: artifact.threadId,
    turnId: artifact.turnId,
    revision: artifact.revision,
    title: artifact.title,
    summary: artifact.summary,
    status: artifact.status,
    createdAt: artifact.createdAt,
    repositoryRoot: artifact.repository.root,
    branch: artifact.repository.branch,
    decisionCount: artifact.decisions.length,
    unresolvedCount: artifact.decisions.filter((decision) => decision.status === "unresolved").length,
  };
}

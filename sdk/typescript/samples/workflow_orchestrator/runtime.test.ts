import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { jsonSchemaFor, reviewOutputSchema } from "./contracts.ts";
import { anchorGitTree, captureGitTree, diffGitTrees, snapshotGit } from "./git.ts";
import { maxSessionsForWorkflow, selectRoute, WorkflowRuntime } from "./runtime.ts";
import type { RoutingGuide } from "./runtime.ts";
import type { PhaseDefinition } from "./phases.ts";
import { describeRunWorkspace, prepareRunWorktree } from "./worktree.ts";

const routing: RoutingGuide = {
  source: "test fixture",
  profiles: [
    {
      id: "terra-high",
      model: "gpt-test-terra",
      reasoningEffort: "high",
      costRank: 1,
      capabilities: { intelligence: 8, codeMechanics: 8, reviewQuality: 8 },
    },
    {
      id: "sol-high",
      model: "gpt-test-sol",
      reasoningEffort: "high",
      costRank: 2,
      capabilities: { intelligence: 10, codeMechanics: 10, reviewQuality: 10 },
    },
  ],
  lanes: {
    architecture: {
      minimum: { intelligence: 8 },
      affinity: [["terra-high"], ["sol-high"]],
      escalation: ["sol-high"],
    },
    implementation: {
      minimum: { codeMechanics: 8 },
      affinity: [["terra-high"], ["sol-high"]],
      escalation: ["sol-high"],
    },
    journeyReview: {
      minimum: { reviewQuality: 8 },
      affinity: [["terra-high"], ["sol-high"]],
      escalation: ["sol-high"],
    },
    codeQualityReview: {
      minimum: { reviewQuality: 8 },
      affinity: [["terra-high"], ["sol-high"]],
      escalation: ["sol-high"],
    },
  },
};

test("bounds and routing prefer the lane fit before escalation", () => {
  assert.equal(maxSessionsForWorkflow(8, 2), 72);
  assert.equal(selectRoute(routing, "implementation").profileId, "terra-high");
});

test("review transitions reject contradictory failure classes", () => {
  assert.ok("properties" in jsonSchemaFor(reviewOutputSchema));
  assert.equal(
    reviewOutputSchema.safeParse({
      decision: "accepted",
      failureClass: "capability",
      summary: "contradictory",
      findings: [],
    }).success,
    false,
  );
  assert.equal(
    reviewOutputSchema.safeParse({
      decision: "needs_repair",
      failureClass: "none",
      summary: "contradictory",
      findings: [],
    }).success,
    false,
  );
});

test("phase evidence is a true tree-to-tree diff including untracked files", () => {
  const repoRoot = createTempRepo();
  try {
    const baseline = captureGitTree(repoRoot);
    fs.writeFileSync(path.join(repoRoot, "tracked.txt"), "after\n");
    fs.writeFileSync(path.join(repoRoot, "untracked.txt"), "new\n");
    const current = captureGitTree(repoRoot);
    anchorGitTree(repoRoot, "refs/codex/workflow-baselines/test/phase-1", baseline);
    const delta = diffGitTrees(repoRoot, baseline, current);
    assert.deepEqual(delta.changedFiles, ["tracked.txt", "untracked.txt"]);
    assert.match(delta.patchExcerpt, /tracked\.txt/);
    assert.notEqual(delta.baselineTree, delta.currentTree);
  } finally {
    fs.rmSync(repoRoot, { recursive: true, force: true });
  }
});

test("run identity cannot escape the isolated worktree root", () => {
  assert.throws(() => describeRunWorkspace("D:/repo", "D:/home", "../escape"), /runId/);
  const repoRoot = createTempRepo();
  try {
    assert.throws(() => describeRunWorkspace(repoRoot, "D:/home", "bad.lock"));
    const workspace = describeRunWorkspace(repoRoot, "D:/home", "safe-run");
    assert.match(workspace.branchName, /^codex\/workflow-/);
  } finally {
    fs.rmSync(repoRoot, { recursive: true, force: true });
  }
});

test("writable workflow workspace is isolated from the source checkout", () => {
  const sourceRepoRoot = createTempRepo();
  const codexHome = fs.mkdtempSync(path.join(os.tmpdir(), "codex-workflow-home-"));
  try {
    fs.writeFileSync(path.join(sourceRepoRoot, "untracked-plan.html"), "<h1>plan</h1>\n");
    const workspace = prepareRunWorktree(
      describeRunWorkspace(sourceRepoRoot, codexHome, "isolation-test"),
    );
    assert.equal(
      fs
        .readFileSync(path.join(workspace.workspaceRoot, "untracked-plan.html"), "utf8")
        .replaceAll("\r\n", "\n"),
      "<h1>plan</h1>\n",
    );
    fs.writeFileSync(path.join(workspace.workspaceRoot, "tracked.txt"), "worktree-only\n");
    assert.equal(fs.readFileSync(path.join(sourceRepoRoot, "tracked.txt"), "utf8"), "before\n");
  } finally {
    fs.rmSync(codexHome, { recursive: true, force: true });
    fs.rmSync(sourceRepoRoot, { recursive: true, force: true });
  }
});

test(
  "resume preserves deadline, counts interruption, and pins routing policy",
  { skip: !process.env.CODEX_EXECUTABLE },
  () => {
    const repoRoot = createTempRepo();
    const stateFile = path.join(repoRoot, "ledger.json");
    const options = {
      repoRoot,
      planRoot: repoRoot,
      stateFile,
      routing,
      maxSessions: 72,
      maxRepairsPerPhase: 2,
      maxSessionMs: 1_000,
      maxRunMs: 60_000,
    };
    try {
      new WorkflowRuntime(options, "resume-test");
      const initial = JSON.parse(fs.readFileSync(stateFile, "utf8"));
      initial.sessionsUsed = 1;
      initial.attempts.push({
        id: "phase-1:implementation:1",
        phaseId: "phase-1",
        role: "implementation",
        status: "running",
        threadId: null,
        route: selectRoute(routing, "implementation"),
        output: null,
        error: null,
        usage: null,
        gitBefore: snapshotGit(repoRoot),
        gitAfter: null,
        startedAt: new Date().toISOString(),
        finishedAt: null,
      });
      fs.writeFileSync(stateFile, JSON.stringify(initial));

      new WorkflowRuntime({ ...options, maxRunMs: 999_999 }, "resume-test");
      const recovered = JSON.parse(fs.readFileSync(stateFile, "utf8"));
      assert.equal(recovered.deadlineAt, initial.deadlineAt);
      assert.equal(recovered.sessionsUsed, 1);
      assert.equal(recovered.attempts[0].status, "interrupted");

      const amendedRouting = structuredClone(routing);
      amendedRouting.source = "changed fixture";
      assert.throws(
        () => new WorkflowRuntime({ ...options, routing: amendedRouting }, "resume-test"),
        /routing policy changed/,
      );
    } finally {
      fs.rmSync(repoRoot, { recursive: true, force: true });
    }
  },
);

test(
  "phase instructions are pinned before any session is reserved",
  { skip: !process.env.CODEX_EXECUTABLE },
  async () => {
    const repoRoot = createTempRepo();
    const stateFile = path.join(os.tmpdir(), `codex-workflow-plan-${process.pid}.json`);
    const planFile = "phase-plan.html";
    const originalPlan = "<h1>Original plan</h1>\n";
    fs.writeFileSync(path.join(repoRoot, planFile), originalPlan);
    const phase: PhaseDefinition = {
      id: "phase-1",
      title: "Pinned plan fixture",
      planFile,
      implementationLane: "implementation",
      objective: "Prove plan pinning",
      requiredPaths: ["tracked.txt"],
    };
    const options = {
      repoRoot,
      planRoot: repoRoot,
      stateFile,
      routing,
      maxSessions: 0,
      maxRepairsPerPhase: 0,
      maxSessionMs: 1_000,
      maxRunMs: 60_000,
    };
    try {
      await assert.rejects(
        new WorkflowRuntime(options, "plan-pin-test").run([phase]),
        /0 sessions/,
      );
      fs.writeFileSync(path.join(repoRoot, planFile), "<h1>Changed later</h1>\n");
      const amendedPhase = { ...phase, objective: "Changed live metadata" };
      await assert.rejects(
        new WorkflowRuntime(options, "plan-pin-test").run([amendedPhase]),
        /0 sessions/,
      );
      const state = JSON.parse(fs.readFileSync(stateFile, "utf8"));
      assert.equal(state.planDocuments["phase-1"].content, originalPlan);
      assert.equal(state.planDocuments["phase-1"].definition.objective, "Prove plan pinning");
      assert.equal(state.sessionsUsed, 0);
    } finally {
      if (fs.existsSync(stateFile)) fs.unlinkSync(stateFile);
      fs.rmSync(repoRoot, { recursive: true, force: true });
    }
  },
);

function createTempRepo(): string {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), "codex-workflow-test-"));
  git(repoRoot, ["init", "--quiet"]);
  git(repoRoot, ["config", "user.email", "workflow-test@example.invalid"]);
  git(repoRoot, ["config", "user.name", "Workflow Test"]);
  fs.writeFileSync(path.join(repoRoot, "tracked.txt"), "before\n");
  git(repoRoot, ["add", "tracked.txt"]);
  git(repoRoot, ["commit", "--quiet", "-m", "fixture"]);
  return repoRoot;
}

function git(repoRoot: string, args: string[]): void {
  execFileSync("git", args, { cwd: repoRoot, stdio: "ignore" });
}

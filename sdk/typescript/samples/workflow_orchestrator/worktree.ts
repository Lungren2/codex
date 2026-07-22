import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

import { captureGitTree } from "./git.ts";

export type RunWorkspace = {
  runId: string;
  sourceRepoRoot: string;
  workspaceRoot: string;
  branchName: string;
  sourceRefName: string;
  baseRef: string | null;
};

export function describeRunWorkspace(
  sourceRepoRoot: string,
  codexHome: string,
  runId: string,
  baseRef: string | null = null,
): RunWorkspace {
  assertRunId(runId);
  const worktreesRoot = path.resolve(
    codexHome,
    "workflow-runs",
    "agent-workflow-runtime",
    "worktrees",
  );
  const workspaceRoot = path.resolve(worktreesRoot, runId);
  const relative = path.relative(worktreesRoot, workspaceRoot);
  if (relative.startsWith("..") || path.isAbsolute(relative)) {
    throw new Error("workflow worktree must remain inside the isolated Codex home");
  }
  const workspace = {
    runId,
    sourceRepoRoot: path.resolve(sourceRepoRoot),
    workspaceRoot,
    branchName: `codex/workflow-${runId}`,
    sourceRefName: `refs/codex/workflow-sources/${runId}`,
    baseRef,
  };
  validateGitNames(workspace);
  return workspace;
}

export function prepareRunWorktree(workspace: RunWorkspace): RunWorkspace {
  if (fs.existsSync(workspace.workspaceRoot)) {
    const actualRoot = git(workspace.workspaceRoot, ["rev-parse", "--show-toplevel"]);
    const actualBranch = git(workspace.workspaceRoot, ["branch", "--show-current"]);
    const sourceCommonDir = git(workspace.sourceRepoRoot, [
      "rev-parse",
      "--path-format=absolute",
      "--git-common-dir",
    ]);
    const workspaceCommonDir = git(workspace.workspaceRoot, [
      "rev-parse",
      "--path-format=absolute",
      "--git-common-dir",
    ]);
    if (
      path.resolve(actualRoot) !== path.resolve(workspace.workspaceRoot) ||
      actualBranch !== workspace.branchName ||
      path.resolve(sourceCommonDir) !== path.resolve(workspaceCommonDir)
    ) {
      throw new Error(`existing workflow workspace is not ${workspace.branchName}`);
    }
    assertAnchoredBaseRef(workspace);
    return workspace;
  }

  fs.mkdirSync(path.dirname(workspace.workspaceRoot), { recursive: true });
  const existingBranch = branchExists(workspace.sourceRepoRoot, workspace.branchName);
  if (existingBranch && !refExists(workspace.sourceRepoRoot, workspace.sourceRefName)) {
    throw new Error(
      `workflow branch exists without its anchored source snapshot: ${workspace.branchName}`,
    );
  }
  if (existingBranch) assertAnchoredBaseRef(workspace);
  const args = existingBranch
    ? ["worktree", "add", workspace.workspaceRoot, workspace.branchName]
    : [
        "worktree",
        "add",
        "-b",
        workspace.branchName,
        workspace.workspaceRoot,
        createSourceSnapshotCommit(workspace),
      ];
  git(workspace.sourceRepoRoot, args);
  return workspace;
}

function refExists(repoRoot: string, refName: string): boolean {
  return (
    spawnSync("git", ["show-ref", "--verify", "--quiet", refName], {
      cwd: repoRoot,
      stdio: "ignore",
    }).status === 0
  );
}

export function assertRunId(runId: string): void {
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,79}$/.test(runId)) {
    throw new Error(
      "runId must be 1-80 characters using letters, digits, dot, underscore, or hyphen",
    );
  }
}

function branchExists(repoRoot: string, branchName: string): boolean {
  return (
    spawnSync("git", ["show-ref", "--verify", "--quiet", `refs/heads/${branchName}`], {
      cwd: repoRoot,
      stdio: "ignore",
    }).status === 0
  );
}

function createSourceSnapshotCommit(workspace: RunWorkspace): string {
  const treeHash = workspace.baseRef
    ? resolveTree(workspace.sourceRepoRoot, workspace.baseRef)
    : captureGitTree(workspace.sourceRepoRoot);
  const identity = {
    ...process.env,
    GIT_AUTHOR_NAME: "Codex Workflow",
    GIT_AUTHOR_EMAIL: "workflow@codex.invalid",
    GIT_COMMITTER_NAME: "Codex Workflow",
    GIT_COMMITTER_EMAIL: "workflow@codex.invalid",
  };
  const commitHash = git(
    workspace.sourceRepoRoot,
    [
      "commit-tree",
      treeHash,
      "-p",
      "HEAD",
      "-m",
      `Workflow source snapshot ${workspace.branchName}`,
    ],
    identity,
  );
  git(workspace.sourceRepoRoot, ["update-ref", workspace.sourceRefName, commitHash]);
  return commitHash;
}

function assertAnchoredBaseRef(workspace: RunWorkspace): void {
  if (!workspace.baseRef) return;
  const requestedTree = resolveTree(workspace.sourceRepoRoot, workspace.baseRef);
  const anchoredTree = resolveTree(workspace.sourceRepoRoot, workspace.sourceRefName);
  if (requestedTree !== anchoredTree) {
    throw new Error(
      `existing workflow source does not match requested base ref ${workspace.baseRef}`,
    );
  }
}

function resolveTree(repoRoot: string, refName: string): string {
  return git(repoRoot, ["rev-parse", "--verify", "--end-of-options", `${refName}^{tree}`]);
}

function validateGitNames(workspace: RunWorkspace): void {
  git(workspace.sourceRepoRoot, ["check-ref-format", "--branch", workspace.branchName]);
  git(workspace.sourceRepoRoot, ["check-ref-format", workspace.sourceRefName]);
  git(workspace.sourceRepoRoot, [
    "check-ref-format",
    `refs/codex/workflow-baselines/${workspace.runId}/phase-probe`,
  ]);
}

function git(repoRoot: string, args: string[], env?: NodeJS.ProcessEnv): string {
  return execFileSync("git", args, {
    cwd: repoRoot,
    encoding: "utf8",
    env,
    stdio: ["ignore", "pipe", "pipe"],
  }).trim();
}

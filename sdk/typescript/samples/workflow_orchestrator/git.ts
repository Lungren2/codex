import { createHash, randomUUID } from "node:crypto";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

export type GitSnapshot = {
  status: string;
  changedFiles: string[];
  diffStat: string;
  diffLines: number;
  patchHash: string;
  patchExcerpt: string;
  fileHashes: Record<string, string>;
};

export type GitDelta = {
  baselineTree: string;
  currentTree: string;
  changedFiles: string[];
  diffStat: string;
  diffLines: number;
  patchHash: string;
  patchExcerpt: string;
};

const MAX_PATCH_CHARS = 4_000;
const MAX_PHASE_PATCH_CHARS = 8_000;

function git(repoRoot: string, args: string[], env?: NodeJS.ProcessEnv): string {
  return execFileSync("git", args, {
    cwd: repoRoot,
    encoding: "utf8",
    env,
    stdio: ["ignore", "pipe", "pipe"],
  }).trim();
}

export function snapshotGit(repoRoot: string): GitSnapshot {
  const status = git(repoRoot, ["status", "--short", "--untracked-files=all"]);
  const patch = git(repoRoot, ["diff", "--no-ext-diff", "--binary", "HEAD", "--", "."]);
  const changedFiles = status
    .split("\n")
    .filter(Boolean)
    .map((line) => line.slice(3).trim())
    .map((file) => (file.includes(" -> ") ? file.split(" -> ").at(-1)! : file))
    .sort();
  const fileHashes = Object.fromEntries(
    changedFiles.map((file) => [file, hashWorkingTreePath(repoRoot, file)]),
  );

  return {
    status,
    changedFiles,
    diffStat: git(repoRoot, ["diff", "--stat", "HEAD", "--", "."]),
    diffLines: patch.length === 0 ? 0 : patch.split("\n").length,
    patchHash: createHash("sha256")
      .update(`${status}\n${patch}\n${JSON.stringify(fileHashes)}`)
      .digest("hex"),
    patchExcerpt: patch.slice(0, MAX_PATCH_CHARS),
    fileHashes,
  };
}

export function captureGitTree(repoRoot: string): string {
  const temporaryIndex = path.join(
    os.tmpdir(),
    `codex-workflow-${process.pid}-${randomUUID()}.index`,
  );
  const env = { ...process.env, GIT_INDEX_FILE: temporaryIndex };
  try {
    git(repoRoot, ["read-tree", "HEAD"], env);
    git(repoRoot, ["add", "--all", "--", "."], env);
    return git(repoRoot, ["write-tree"], env);
  } finally {
    if (fs.existsSync(temporaryIndex)) fs.unlinkSync(temporaryIndex);
  }
}

export function anchorGitTree(repoRoot: string, refName: string, treeHash: string): void {
  git(repoRoot, ["check-ref-format", refName]);
  git(repoRoot, ["update-ref", refName, treeHash]);
}

export function diffGitTrees(
  repoRoot: string,
  baselineTree: string,
  currentTree: string,
): GitDelta {
  const patch = git(repoRoot, [
    "diff",
    "--no-ext-diff",
    "--binary",
    baselineTree,
    currentTree,
    "--",
    ".",
  ]);
  const changedFiles = git(repoRoot, [
    "diff",
    "--name-only",
    "-z",
    baselineTree,
    currentTree,
    "--",
    ".",
  ])
    .split("\0")
    .filter(Boolean)
    .sort();
  return {
    baselineTree,
    currentTree,
    changedFiles,
    diffStat: git(repoRoot, ["diff", "--stat", baselineTree, currentTree, "--", "."]),
    diffLines: patch.length === 0 ? 0 : patch.split("\n").length,
    patchHash: createHash("sha256").update(patch).digest("hex"),
    patchExcerpt: patch.slice(0, MAX_PHASE_PATCH_CHARS),
  };
}

export function sameGitState(left: GitSnapshot, right: GitSnapshot): boolean {
  return left.patchHash === right.patchHash;
}

function hashWorkingTreePath(repoRoot: string, relativePath: string): string {
  const absolutePath = path.join(repoRoot, relativePath);
  if (!fs.existsSync(absolutePath)) return "missing";
  const stat = fs.statSync(absolutePath);
  if (!stat.isFile()) return `non-file:${stat.mode}`;
  return createHash("sha256").update(fs.readFileSync(absolutePath)).digest("hex");
}

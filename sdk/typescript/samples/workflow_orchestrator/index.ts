#!/usr/bin/env -S NODE_NO_WARNINGS=1 pnpm ts-node-esm --files

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { phases } from "./phases.ts";
import type { PhaseId } from "./phases.ts";
import {
  assertRoutingGuide,
  maxSessionsForWorkflow,
  selectRoute,
  WorkflowRuntime,
} from "./runtime.ts";
import type { RoutingGuide } from "./runtime.ts";
import { describeRunWorkspace, prepareRunWorktree } from "./worktree.ts";

const scriptRoot = path.dirname(fileURLToPath(import.meta.url));
const defaultRepoRoot = path.resolve(scriptRoot, "../../../..");
const args = parseArgs(process.argv.slice(2));
const sourceRepoRoot = path.resolve(args.repo ?? defaultRepoRoot);
const routingPath = args.routing ? path.resolve(args.routing) : null;
const startIndex = args.from ? phases.findIndex((phase) => phase.id === args.from) : 0;

if (!routingPath || !fs.existsSync(routingPath)) {
  throw new Error("--routing must name an existing host-owned routing JSON file");
}
if (startIndex < 0) {
  throw new Error(`unknown --from phase: ${args.from}`);
}
const selectedPhases = phases.slice(startIndex);

const routing = JSON.parse(fs.readFileSync(routingPath, "utf8")) as RoutingGuide;
assertRoutingGuide(routing);
const runId = args.runId ?? new Date().toISOString().replaceAll(/[:.]/g, "-");
const codexHome = process.env.CODEX_HOME ?? path.join(os.homedir(), ".codex");
const stateFile = path.join(codexHome, "workflow-runs", "agent-workflow-runtime", `${runId}.json`);
const workspace = describeRunWorkspace(sourceRepoRoot, codexHome, runId);
const maxRepairsPerPhase = 2;
const maxSessions = maxSessionsForWorkflow(phases.length, maxRepairsPerPhase);
const maxSessionMs = 45 * 60 * 1_000;
const maxRunMs = 8 * 60 * 60 * 1_000;

if (!args.execute) {
  console.log(
    JSON.stringify(
      {
        sourceRepoRoot,
        workspace,
        routingPath,
        runId,
        stateFile,
        limits: { maxSessions, maxRepairsPerPhase, maxSessionMs, maxRunMs },
        routePreview: Object.fromEntries(
          (["architecture", "implementation", "journeyReview", "codeQualityReview"] as const).map(
            (lane) => [lane, selectRoute(routing, lane)],
          ),
        ),
        phases: selectedPhases,
      },
      null,
      2,
    ),
  );
  console.log("Dry run only. Pass --execute to start bounded Codex sessions.");
  process.exit(0);
}

prepareRunWorktree(workspace);
const runtime = new WorkflowRuntime(
  {
    repoRoot: workspace.workspaceRoot,
    planRoot: scriptRoot,
    stateFile,
    routing,
    maxSessions,
    maxRepairsPerPhase,
    maxSessionMs,
    maxRunMs,
  },
  runId,
);

await runtime.run(selectedPhases);
console.log(`Workflow completed. Ledger: ${stateFile}`);

type CliArgs = {
  execute: boolean;
  repo?: string;
  routing?: string;
  runId?: string;
  from?: PhaseId;
};

function parseArgs(values: string[]): CliArgs {
  const parsed: CliArgs = { execute: false };
  for (let index = 0; index < values.length; index += 1) {
    const value = values[index];
    if (value === "--execute") {
      parsed.execute = true;
      continue;
    }
    if (value === "--repo" || value === "--routing" || value === "--run-id" || value === "--from") {
      const argument = values[index + 1];
      if (!argument) throw new Error(`${value} requires a value`);
      index += 1;
      if (value === "--repo") parsed.repo = argument;
      if (value === "--routing") parsed.routing = argument;
      if (value === "--run-id") parsed.runId = argument;
      if (value === "--from") parsed.from = argument as PhaseId;
      continue;
    }
    throw new Error(`unknown argument: ${value}`);
  }
  return parsed;
}

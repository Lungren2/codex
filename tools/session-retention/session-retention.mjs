#!/usr/bin/env node
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  appendLog,
  createRuntime,
  discoverActiveSessionIds,
  discoverArchivedSessions,
  discoverSavedSessions,
  getDueStatus,
  installScheduler as installSharedScheduler,
  runCommand,
  schedulerCommand,
  schedulerOptionsFromRuntime,
  uninstallScheduler as uninstallSharedScheduler,
  withLock,
  writeJsonAtomic,
} from "../lib/codex-maintenance.mjs";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const DEFAULT_CONFIG = {
  codexHome: "../..",
  codexCommand: "codex",
  codexCommandArgs: [],
  nodeCommand: "node",
  archive: { scheduleTime: "08:00", intervalDays: 1, inactiveHours: 24 },
  delete: { scheduleTime: "08:00", intervalDays: 1, retentionDays: 2 },
  lockStaleMinutes: 120,
  schedulerName: "CodexSessionRetention",
};

const DEFAULT_STATE = {
  lastArchiveRunAt: null,
  lastDeleteRunAt: null,
  archivedSessions: {},
  deletedSessions: {},
};

export function parseArgs(argv) {
  const args = { command: argv[0] ?? "help", dryRun: false, yes: false, json: false, configPath: null };
  for (let i = 1; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--dry-run") args.dryRun = true;
    else if (arg === "--yes") args.yes = true;
    else if (arg === "--json") args.json = true;
    else if (arg === "--config") args.configPath = argv[++i];
    else throw new Error(`Unknown option: ${arg}`);
  }
  return args;
}

export async function loadRuntime(configPath = null) {
  return await createRuntime({
    configPath,
    defaultConfig: DEFAULT_CONFIG,
    defaultState: DEFAULT_STATE,
    scriptPath: __filename,
    lockName: "session-retention.lock",
  });
}

export { getDueStatus };

export async function getArchiveCandidates(runtime, now = new Date()) {
  const sessions = await discoverSavedSessions(runtime.codexHome);
  const archived = await discoverArchivedSessions(runtime.codexHome);
  const active = await discoverActiveSessionIds(runtime.codexHome);
  const inactiveBefore = new Date(now.valueOf() - runtime.config.archive.inactiveHours * 60 * 60 * 1000);
  return [...sessions.values()]
    .filter((session) => session.path)
    .filter((session) => !archived.has(session.id))
    .filter((session) => !active.has(session.id))
    .filter((session) => session.updatedAt && session.updatedAt <= inactiveBefore)
    .sort((a, b) => a.updatedAt - b.updatedAt);
}

export async function getDeleteCandidates(runtime, now = new Date()) {
  const archived = await discoverArchivedSessions(runtime.codexHome);
  const active = await discoverActiveSessionIds(runtime.codexHome);
  const retentionMs = runtime.config.delete.retentionDays * 24 * 60 * 60 * 1000;
  const candidates = [];
  for (const [id, info] of archived) {
    const tracked = runtime.state.archivedSessions?.[id];
    if (!tracked?.archivedAt || active.has(id)) continue;
    const archivedAt = new Date(tracked.archivedAt);
    if (!Number.isNaN(archivedAt.valueOf()) && now.valueOf() - archivedAt.valueOf() >= retentionMs) {
      candidates.push({ ...info, archivedAt });
    }
  }
  return candidates.sort((a, b) => a.archivedAt - b.archivedAt);
}

export async function archiveEligible(runtime, options = {}) {
  const now = options.now ?? new Date();
  const dryRun = options.dryRun ?? true;
  const candidates = await getArchiveCandidates(runtime, now);
  const results = [];
  for (const [index, session] of candidates.entries()) {
    if (dryRun) {
      const record = { id: session.id, action: "archive", dryRun: true, ok: true, updatedAt: session.updatedAt.toISOString() };
      results.push(record);
      options.onProgress?.({ current: index + 1, total: candidates.length, record });
      continue;
    }
    const result = runCommand(runtime.config.codexCommand, [...runtime.config.codexCommandArgs, "archive", session.id], runtime.codexHome);
    const record = { id: session.id, action: "archive", dryRun: false, ...result };
    results.push(record);
    await appendLog(runtime, record);
    if (result.ok) {
      runtime.state.archivedSessions[session.id] = {
        archivedAt: now.toISOString(),
        updatedAt: session.updatedAt.toISOString(),
        threadName: session.name ?? null,
      };
      await writeJsonAtomic(runtime.statePath, runtime.state);
    }
    options.onProgress?.({ current: index + 1, total: candidates.length, record });
  }
  if (!dryRun) {
    runtime.state.lastArchiveRunAt = now.toISOString();
    await writeJsonAtomic(runtime.statePath, runtime.state);
  }
  return { dryRun, candidates: candidates.length, results };
}

export async function deleteEligible(runtime, options = {}) {
  const now = options.now ?? new Date();
  const dryRun = options.dryRun ?? true;
  const candidates = await getDeleteCandidates(runtime, now);
  const results = [];
  for (const [index, session] of candidates.entries()) {
    if (dryRun) {
      const record = { id: session.id, action: "delete", dryRun: true, ok: true, archivedAt: session.archivedAt.toISOString() };
      results.push(record);
      options.onProgress?.({ current: index + 1, total: candidates.length, record });
      continue;
    }
    const result = runCommand(runtime.config.codexCommand, [...runtime.config.codexCommandArgs, "delete", "--force", session.id], runtime.codexHome);
    const record = { id: session.id, action: "delete", dryRun: false, ...result };
    results.push(record);
    await appendLog(runtime, record);
    if (result.ok) {
      runtime.state.deletedSessions[session.id] = {
        deletedAt: now.toISOString(),
        archivedAt: session.archivedAt.toISOString(),
      };
      delete runtime.state.archivedSessions[session.id];
      await writeJsonAtomic(runtime.statePath, runtime.state);
    }
    options.onProgress?.({ current: index + 1, total: candidates.length, record });
  }
  if (!dryRun) {
    runtime.state.lastDeleteRunAt = now.toISOString();
    await writeJsonAtomic(runtime.statePath, runtime.state);
  }
  return { dryRun, candidates: candidates.length, results };
}

export async function runDue(runtime, now = new Date(), options = {}) {
  return await withLock(runtime, async () => {
    const archiveDue = getDueStatus(
      now,
      runtime.state.lastArchiveRunAt,
      runtime.config.archive.scheduleTime,
      runtime.config.archive.intervalDays,
    );
    const deleteDue = getDueStatus(
      now,
      runtime.state.lastDeleteRunAt,
      runtime.config.delete.scheduleTime,
      runtime.config.delete.intervalDays,
    );
    const results = { archiveDue, deleteDue, archive: null, delete: null };
    if (archiveDue.due) {
      results.archive = await archiveEligible(runtime, { dryRun: false, now, onProgress: options.onProgress });
    }
    if (deleteDue.due) {
      results.delete = await deleteEligible(runtime, { dryRun: false, now, onProgress: options.onProgress });
    }
    if (!archiveDue.due && !deleteDue.due) await appendLog(runtime, { action: "run", ok: true, skipped: true });
    return results;
  });
}

export async function getStatus(runtime, now = new Date()) {
  const archiveCandidates = await getArchiveCandidates(runtime, now);
  const deleteCandidates = await getDeleteCandidates(runtime, now);
  return {
    codexHome: runtime.codexHome,
    archiveDue: getDueStatus(now, runtime.state.lastArchiveRunAt, runtime.config.archive.scheduleTime, runtime.config.archive.intervalDays),
    deleteDue: getDueStatus(now, runtime.state.lastDeleteRunAt, runtime.config.delete.scheduleTime, runtime.config.delete.intervalDays),
    archiveCandidates: archiveCandidates.length,
    deleteCandidates: deleteCandidates.length,
    trackedArchivedSessions: Object.keys(runtime.state.archivedSessions ?? {}).length,
    trackedDeletedSessions: Object.keys(runtime.state.deletedSessions ?? {}).length,
  };
}

export async function installScheduler(runtime) {
  return await installSharedScheduler(retentionSchedulerOptions(runtime));
}

export async function uninstallScheduler(runtime) {
  return await uninstallSharedScheduler(retentionSchedulerOptions(runtime));
}

function retentionSchedulerOptions(runtime) {
  return schedulerOptionsFromRuntime(runtime, {
    description: "Codex session archive/delete retention",
    scheduleTime: runtime.config.archive.scheduleTime,
    linuxUnitName: "codex-session-retention",
    macLabel: "com.codex.session-retention",
  });
}

function printResult(value, json = false) {
  if (json) {
    console.log(JSON.stringify(value, null, 2));
    return;
  }
  if (value.results) {
    console.log(`${value.dryRun ? "Dry run" : "Live run"}: ${value.candidates} candidate(s)`);
    if (value.progressStreamed) {
      const failed = value.results.filter((result) => !result.ok).length;
      console.log(`Completed: ${value.results.length}/${value.candidates} processed, ${failed} failed`);
      return;
    }
    for (const result of value.results) {
      console.log(`${result.ok ? "OK" : "FAIL"} ${result.action} ${result.id}${result.dryRun ? " (dry-run)" : ""}`);
      if (!result.ok && (result.stderr || result.error)) console.log(result.stderr || result.error);
    }
  } else {
    console.log(JSON.stringify(value, null, 2));
  }
}

function printProgress({ current, total, record }) {
  console.log(`[${current}/${total}] ${record.ok ? "OK" : "FAIL"} ${record.action} ${record.id}${record.dryRun ? " (dry-run)" : ""}`);
  if (!record.ok && (record.stderr || record.error)) console.log(record.stderr || record.error);
}

function help() {
  const defaultRuntime = {
    config: DEFAULT_CONFIG,
    scriptPath: __filename,
    configPath: path.join(__dirname, "config.json"),
    codexHome: path.resolve(__dirname, "../.."),
  };
  return `Codex session retention

Usage:
  node tools/session-retention/session-retention.mjs status [--json]
  node tools/session-retention/session-retention.mjs run [--json]
  node tools/session-retention/session-retention.mjs archive [--dry-run|--yes] [--json]
  node tools/session-retention/session-retention.mjs delete [--dry-run|--yes] [--json]
  node tools/session-retention/session-retention.mjs install-scheduler
  node tools/session-retention/session-retention.mjs uninstall-scheduler

Manual archive/delete default to dry-run. Pass --yes for live changes. The scheduled run command is live by design.
Scheduler command: ${schedulerCommand(retentionSchedulerOptions(defaultRuntime))}
`;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.command === "help" || args.command === "--help" || args.command === "-h") {
    console.log(help());
    return;
  }
  const runtime = await loadRuntime(args.configPath);
  const onProgress = args.json ? null : printProgress;
  let result;
  if (args.command === "status") result = await getStatus(runtime);
  else if (args.command === "run") result = await runDue(runtime, new Date(), { onProgress });
  else if (args.command === "archive") result = await archiveEligible(runtime, { dryRun: !args.yes || args.dryRun, onProgress });
  else if (args.command === "delete") result = await deleteEligible(runtime, { dryRun: !args.yes || args.dryRun, onProgress });
  else if (args.command === "install-scheduler") result = await installScheduler(runtime);
  else if (args.command === "uninstall-scheduler") result = await uninstallScheduler(runtime);
  else throw new Error(`Unknown command: ${args.command}`);
  if (!args.json && result?.results && onProgress) result.progressStreamed = true;
  printResult(result, args.json);
  if (result?.ok === false) process.exitCode = result.status || 1;
}

if (process.argv[1] && path.resolve(process.argv[1]) === __filename) {
  main().catch((error) => {
    console.error(error.stack || error.message);
    process.exitCode = 1;
  });
}

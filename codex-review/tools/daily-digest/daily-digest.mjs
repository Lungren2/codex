#!/usr/bin/env node
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  addDays,
  appendLog,
  compareYmd,
  createRuntime,
  dateFromYmd,
  discoverActiveSessionIds,
  discoverArchivedSessions,
  discoverSavedSessions,
  getDueStatus,
  installScheduler as installSharedScheduler,
  latestScheduledAtOrBefore,
  previousYmd,
  runCommand,
  schedulerCommand,
  schedulerOptionsFromRuntime,
  uninstallScheduler as uninstallSharedScheduler,
  walkFiles,
  withLock,
  writeJsonAtomic,
  ymdFromDate,
} from "../lib/codex-maintenance.mjs";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const DEFAULT_CONFIG = {
  codexHome: "../..",
  outputDir: "~/Desktop",
  outputFilename: "Codex Daily Digest.html",
  digestDirName: "Codex Daily Digests",
  outputFormat: "html",
  tmpDir: "tmp",
  codexCommand: "codex",
  codexCommandArgs: [],
  nodeCommand: "node",
  model: null,
  profile: null,
  scheduleTime: "08:30",
  intervalDays: 1,
  maxCatchUpDays: 14,
  includeArchivedSessions: true,
  includeRetentionLogs: true,
  inferenceTimeoutMs: 180000,
  lockStaleMinutes: 120,
  schedulerName: "CodexDailyDigest",
};

const DEFAULT_STATE = {
  lastRunAt: null,
  generatedDigests: {},
  lastFailure: null,
};

export function parseArgs(argv) {
  const args = { command: argv[0] ?? "help", date: null, dryRun: false, yes: false, json: false, configPath: null };
  for (let i = 1; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--date") args.date = argv[++i];
    else if (arg === "--dry-run") args.dryRun = true;
    else if (arg === "--yes") args.yes = true;
    else if (arg === "--json") args.json = true;
    else if (arg === "--config") args.configPath = argv[++i];
    else throw new Error(`Unknown option: ${arg}`);
  }
  return args;
}

export async function loadRuntime(configPath = null) {
  const runtime = await createRuntime({
    configPath,
    defaultConfig: DEFAULT_CONFIG,
    defaultState: DEFAULT_STATE,
    scriptPath: __filename,
    lockName: "daily-digest.lock",
  });
  runtime.outputDir = resolveConfiguredPath(runtime.toolDir, runtime.config.outputDir);
  runtime.digestDir = path.join(runtime.outputDir, runtime.config.digestDirName ?? "Codex Daily Digests");
  runtime.indexPath = path.join(runtime.outputDir, runtime.config.outputFilename ?? "Codex Daily Digest.html");
  runtime.tmpDir = path.resolve(runtime.toolDir, runtime.config.tmpDir);
  return runtime;
}

export function digestDateForScheduledWindow(now, scheduleTime) {
  return previousYmd(ymdFromDate(latestScheduledAtOrBefore(now, scheduleTime)));
}

export function getDueDigestDates(runtime, now = new Date()) {
  const dueStatus = getDueStatus(now, runtime.state.lastRunAt, runtime.config.scheduleTime, runtime.config.intervalDays);
  if (!dueStatus.due) return [];
  const latestDigestDate = digestDateForScheduledWindow(now, runtime.config.scheduleTime);
  const generated = runtime.state.generatedDigests ?? {};
  const dates = [];
  let cursor = latestDigestDate;
  for (let i = 0; i < runtime.config.maxCatchUpDays; i += 1) {
    if (!generated[cursor]) dates.push(cursor);
    cursor = previousYmd(cursor);
  }
  return dates.sort(compareYmd);
}

export async function getStatus(runtime, now = new Date()) {
  const dueDates = getDueDigestDates(runtime, now);
  const evidence = dueDates.length ? await buildEvidenceBundle(runtime, dueDates.at(-1)) : null;
  return {
    codexHome: runtime.codexHome,
    outputDir: runtime.outputDir,
    digestDir: runtime.digestDir,
    indexPath: runtime.indexPath,
    due: dueDates.length > 0,
    dueDates,
    nextDue: getDueStatus(now, runtime.state.lastRunAt, runtime.config.scheduleTime, runtime.config.intervalDays),
    generatedDigests: Object.keys(runtime.state.generatedDigests ?? {}).length,
    candidateSessions: evidence?.sessions.length ?? 0,
    lastFailure: runtime.state.lastFailure,
  };
}

export async function runDue(runtime, now = new Date(), options = {}) {
  return await withLock(runtime, async () => {
    const dueDates = getDueDigestDates(runtime, now);
    const results = [];
    for (const date of dueDates) {
      results.push(await generateDigest(runtime, { date, dryRun: options.dryRun ?? false, now }));
    }
    if (results.length === 0) await appendLog(runtime, { action: "daily-digest-run", ok: true, skipped: true });
    if (!(options.dryRun ?? false)) {
      runtime.state.lastRunAt = now.toISOString();
      await writeJsonAtomic(runtime.statePath, runtime.state);
    }
    return { dueDates, results, dryRun: options.dryRun ?? false };
  });
}

export async function generateDigest(runtime, options) {
  const date = options.date;
  if (!date) throw new Error("generate requires --date YYYY-MM-DD");
  dateFromYmd(date);
  const now = options.now ?? new Date();
  const dryRun = options.dryRun ?? true;
  const evidence = await buildEvidenceBundle(runtime, date);
  const fallbackMarkdown = renderFallbackDigest(evidence);
  const outputPath = digestPath(runtime, date);
  if (dryRun) {
    return {
      date,
      dryRun: true,
      ok: true,
      sessions: evidence.sessions.length,
      outputPath,
      fallbackMarkdown,
    };
  }

  await fs.mkdir(runtime.outputDir, { recursive: true });
  await fs.mkdir(runtime.digestDir, { recursive: true });
  await fs.mkdir(runtime.tmpDir, { recursive: true });
  const evidencePath = path.join(runtime.tmpDir, `${date}.evidence.json`);
  const promptPath = path.join(runtime.tmpDir, `${date}.prompt.md`);
  const summaryPath = path.join(runtime.tmpDir, `${date}.summary.md`);
  const prompt = buildDigestPrompt(evidence);
  await fs.writeFile(evidencePath, `${JSON.stringify(evidence, null, 2)}\n`, "utf8");
  await fs.writeFile(promptPath, prompt, "utf8");

  const inference = runInference(runtime, prompt, summaryPath);
  let usedFallback = false;
  let markdown = fallbackMarkdown;
  if (inference.ok) {
    try {
      markdown = await fs.readFile(summaryPath, "utf8");
    } catch {
      markdown = inference.stdout || fallbackMarkdown;
    }
  } else {
    usedFallback = true;
    runtime.state.lastFailure = {
      failedAt: now.toISOString(),
      date,
      error: inference.error,
      stderr: inference.stderr,
      status: inference.status,
    };
  }

  await fs.writeFile(outputPath, renderDigestOutput(runtime, date, markdown, evidence, { inferenceOk: inference.ok, usedFallback }), "utf8");
  runtime.state.generatedDigests[date] = {
    generatedAt: now.toISOString(),
    outputPath,
    indexPath: runtime.indexPath,
    evidencePath,
    promptPath,
    summaryPath,
    format: runtime.config.outputFormat,
    sessions: evidence.sessions.length,
    inferenceOk: inference.ok,
    usedFallback,
  };
  if (inference.ok) runtime.state.lastFailure = null;
  await writeJsonAtomic(runtime.statePath, runtime.state);
  await writeDigestIndex(runtime, now);
  await appendLog(runtime, { action: "daily-digest-generate", ok: true, date, outputPath, sessions: evidence.sessions.length, inferenceOk: inference.ok });
  return { date, dryRun: false, ok: true, sessions: evidence.sessions.length, outputPath, indexPath: runtime.indexPath, evidencePath, inferenceOk: inference.ok, usedFallback };
}

export async function buildEvidenceBundle(runtime, date) {
  const { start, end } = dayWindow(date);
  const active = await discoverActiveSessionIds(runtime.codexHome);
  const saved = await discoverSavedSessions(runtime.codexHome);
  const archived = runtime.config.includeArchivedSessions ? await discoverArchivedSessions(runtime.codexHome) : new Map();
  const byId = new Map();
  for (const session of saved.values()) byId.set(session.id, session);
  for (const session of archived.values()) byId.set(session.id, { ...byId.get(session.id), ...session, archived: true });

  const sessions = [];
  for (const session of byId.values()) {
    const updatedAt = session.updatedAt ?? session.mtime;
    if (!session.path || !updatedAt || updatedAt < start || updatedAt >= end) continue;
    sessions.push(await parseSessionEvidence(session, active.has(session.id)));
  }
  sessions.sort((a, b) => String(a.updatedAt).localeCompare(String(b.updatedAt)));

  return {
    digestDate: date,
    generatedAt: new Date().toISOString(),
    codexHome: runtime.codexHome,
    window: { start: start.toISOString(), end: end.toISOString() },
    sessions,
    retentionLogEvents: runtime.config.includeRetentionLogs ? await readRetentionLogEvents(runtime, date) : [],
  };
}

export async function installScheduler(runtime) {
  return await installSharedScheduler(digestSchedulerOptions(runtime));
}

export async function uninstallScheduler(runtime) {
  return await uninstallSharedScheduler(digestSchedulerOptions(runtime));
}

function runInference(runtime, prompt, outputPath) {
  const args = [
    ...runtime.config.codexCommandArgs,
    "exec",
    ...optionalFlag("-m", runtime.config.model),
    ...optionalFlag("-p", runtime.config.profile),
    "--ephemeral",
    "--skip-git-repo-check",
    "--sandbox",
    "read-only",
    "-C",
    runtime.codexHome,
    "-o",
    outputPath,
    "-",
  ];
  return runCommand(runtime.config.codexCommand, args, runtime.codexHome, {
    input: prompt,
    timeoutMs: runtime.config.inferenceTimeoutMs,
  });
}

async function parseSessionEvidence(session, active) {
  const text = await fs.readFile(session.path, "utf8");
  const evidence = {
    id: session.id,
    title: session.name ?? null,
    path: session.path,
    updatedAt: (session.updatedAt ?? session.mtime)?.toISOString?.() ?? null,
    archived: Boolean(session.archived),
    active,
    cwd: null,
    cliVersion: null,
    userPrompts: [],
    assistantMessages: [],
    commands: [],
    failures: [],
    files: [],
  };
  const fileHits = new Set();
  let eventIndex = 0;
  for (const line of text.split(/\r?\n/)) {
    if (!line.trim()) continue;
    eventIndex += 1;
    let row;
    try {
      row = JSON.parse(line);
    } catch {
      continue;
    }
    if (row.type === "session_meta") {
      evidence.cwd = row.payload?.cwd ?? evidence.cwd;
      evidence.cliVersion = row.payload?.cli_version ?? evidence.cliVersion;
      continue;
    }
    const compact = compactEvent(row);
    if (!compact) continue;
    for (const hit of extractPathHints(compact.text)) fileHits.add(hit);
    if (compact.kind === "user") evidence.userPrompts.push(withEvidenceId(eventIndex, compact.text));
    else if (compact.kind === "assistant") evidence.assistantMessages.push(withEvidenceId(eventIndex, compact.text));
    else if (compact.kind === "command") evidence.commands.push({ evidenceId: `e${eventIndex}`, command: compact.text });
    if (compact.failure) evidence.failures.push(withEvidenceId(eventIndex, compact.text));
  }
  evidence.files = [...fileHits].sort();
  return trimEvidence(evidence);
}

function compactEvent(row) {
  const text = sanitizeText(extractText(row));
  if (!text || isSessionScaffolding(text)) return null;
  const kind = inferKind(row);
  return {
    kind,
    text: kind === "command" ? extractCommand(row) ?? text : text,
    // Assistant narration often contains words such as "failed" while explaining
    // a resolved problem. Event messages are the reliable operational failure feed.
    failure: row.type === "event_msg" && isOperationalFailure(text),
  };
}

function isSessionScaffolding(text) {
  return /^(<permissions instructions>|# AGENTS\.md instructions|## Skills|<environment_context>|<plugins_instructions>|# Context from my IDE setup:|You are a Tailwind CSS v4 documentation subagent)/i.test(text);
}

function isOperationalFailure(text) {
  return /\b(error|failed|failure|fatal|exception|exit code [1-9]|status [1-9])\b/i.test(text)
    && !isSessionScaffolding(text);
}

function inferKind(row) {
  const role = row.payload?.role ?? row.role ?? row.item?.role ?? row.payload?.item?.role;
  if (role === "user") return "user";
  if (role === "assistant") return "assistant";
  const type = row.payload?.type ?? row.type;
  if (["custom_tool_call", "function_call"].includes(type)) return "command";
  if (row.type === "event_msg") return "assistant";
  return "assistant";
}

function extractCommand(row) {
  const payload = row.payload ?? {};
  const source = String(payload.input ?? payload.arguments ?? "").replaceAll('\\"', '"');
  const match = /["']cmd["']\s*:\s*["']((?:\\.|[^"'])*)["']/.exec(source);
  if (!match) return null;
  try {
    return JSON.parse(`"${match[1]}"`);
  } catch {
    return match[1].replace(/\\([\\"'])/g, "$1");
  }
}

function extractText(value, key = "") {
  if (value == null) return "";
  if (typeof value === "string") return value;
  if (typeof value !== "object") return "";
  if (["base_instructions", "instructions"].includes(key)) return "";
  if (Array.isArray(value)) return value.map((item) => extractText(item)).filter(Boolean).join("\n");
  const pieces = [];
  for (const [childKey, childValue] of Object.entries(value)) {
    if (["base_instructions", "instructions"].includes(childKey)) continue;
    if (["text", "content", "message", "cmd", "command", "input", "arguments", "stdout", "stderr"].includes(childKey)) {
      const childText = extractText(childValue, childKey);
      if (childText) pieces.push(childText);
    } else if (typeof childValue === "object") {
      const childText = extractText(childValue, childKey);
      if (childText) pieces.push(childText);
    }
  }
  return pieces.join("\n");
}

function sanitizeText(text) {
  return String(text)
    .replace(/\r/g, "")
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .join("\n")
    .slice(0, 4000);
}

function withEvidenceId(index, text) {
  return { evidenceId: `e${index}`, text };
}

function trimEvidence(evidence) {
  return {
    ...evidence,
    userPrompts: evidence.userPrompts.slice(-8),
    assistantMessages: evidence.assistantMessages.slice(-8),
    commands: evidence.commands.slice(-20),
    failures: evidence.failures.slice(-20),
    files: evidence.files.slice(0, 80),
  };
}

function extractPathHints(text) {
  const hits = new Set();
  const re = /(?:[A-Za-z]:\\[^\s"'<>]+|(?:\.{1,2}[\\/])?[A-Za-z0-9_.-]+(?:[\\/][A-Za-z0-9_.-]+)+)/g;
  for (const match of text.matchAll(re)) hits.add(match[0].replace(/[),.;:]+$/, ""));
  return hits;
}

async function readRetentionLogEvents(runtime, date) {
  const logsDir = path.resolve(runtime.toolDir, "..", "session-retention", "logs");
  const files = await walkFiles(logsDir, (file) => path.basename(file).startsWith(date) && file.endsWith(".log"));
  const events = [];
  for (const file of files) {
    const text = await fs.readFile(file, "utf8");
    for (const line of text.split(/\r?\n/)) {
      if (!line.trim()) continue;
      try {
        events.push({ file, ...JSON.parse(line) });
      } catch {
        events.push({ file, raw: line });
      }
    }
  }
  return events.slice(-100);
}

function buildDigestPrompt(evidence) {
  return `You are generating a Codex daily digest for ${evidence.digestDate}.

Use only the evidence JSON below. Concrete claims must cite evidence ids like [session-id:e12] or retention log timestamps. Separate inference from directly observed facts. Keep it concise and operational. This is a morning briefing, not a session transcript: lead with substantive implementation, decisions, validation, risks, and next actions. Treat disposable documentation, summarisation, retention, or temporary-workspace sessions as supporting activity; do not present them as the day's main work unless the evidence shows no substantive work. When no substantive work is captured, say that plainly in Topline and keep supporting activity compact.

Required Markdown sections:
## Topline
## Work By Workspace
## Sessions
## Commands And Validation
## Failures And Follow-Ups

Evidence JSON:
${JSON.stringify(evidence, null, 2)}
`;
}

function renderFallbackDigest(evidence) {
  const substantiveSessions = evidence.sessions.filter((session) => !isSupportingSession(session));
  const supportingSessions = evidence.sessions.length - substantiveSessions.length;
  const lines = [
    `# Codex Daily Digest - ${evidence.digestDate}`,
    "",
    "## Topline",
    substantiveSessions.length
      ? `- Substantive sessions touched: ${substantiveSessions.length}`
      : "- No substantive implementation session was captured for this date.",
    supportingSessions ? `- Supporting or temporary sessions: ${supportingSessions}` : null,
    evidence.retentionLogEvents.length ? `- Retention log events: ${evidence.retentionLogEvents.length}` : null,
    "",
    "## Work By Workspace",
  ];
  const byCwd = new Map();
  for (const session of substantiveSessions) {
    const key = session.cwd ?? "(unknown cwd)";
    byCwd.set(key, (byCwd.get(key) ?? 0) + 1);
  }
  if (byCwd.size === 0) lines.push("- No substantive workspace evidence found for this date.");
  for (const [cwd, count] of byCwd) lines.push(`- ${cwd}: ${count} session(s)`);

  lines.push("", "## Sessions");
  if (substantiveSessions.length === 0) lines.push("- Supporting sessions are retained in the evidence appendix but omitted from this first-read summary.");
  for (const session of substantiveSessions) {
    lines.push(`- ${session.title ?? session.id} (${session.id})`);
    lines.push(`  - Updated: ${session.updatedAt ?? "unknown"}`);
    if (session.userPrompts[0]) lines.push(`  - Latest user prompt: ${session.userPrompts.at(-1).text.split("\n")[0]}`);
    if (session.failures.length) lines.push(`  - Failures: ${session.failures.length}`);
  }

  lines.push("", "## Commands And Validation");
  const commands = substantiveSessions.flatMap((session) => session.commands.map((command) => ({ session, command })));
  if (commands.length === 0) lines.push("- No command evidence extracted.");
  for (const { session, command } of commands.slice(-30)) lines.push(`- ${session.id}:${command.evidenceId} ${command.command.split("\n")[0]}`);

  lines.push("", "## Failures And Follow-Ups");
  const failures = substantiveSessions.flatMap((session) => session.failures.map((failure) => ({ session, failure })));
  if (failures.length === 0) lines.push("- No failure evidence extracted.");
  for (const { session, failure } of failures.slice(-30)) lines.push(`- ${session.id}:${failure.evidenceId} ${failure.text.split("\n")[0]}`);
  return `${lines.join("\n")}\n`;
}

function isSupportingSession(session) {
  const cwd = session.cwd ?? "";
  const prompts = session.userPrompts.map((prompt) => prompt.text).join("\n");
  return /[\\/]AppData[\\/]Local[\\/]Temp[\\/]|tailwind-doc-agent-/i.test(cwd)
    || /Tailwind CSS v4 documentation subagent/i.test(prompts);
}

function dayWindow(date) {
  const start = dateFromYmd(date);
  const end = addDays(start, 1);
  return { start, end };
}

function digestPath(runtime, date) {
  return path.join(runtime.digestDir, `${date}.html`);
}

function optionalFlag(flag, value) {
  return value ? [flag, value] : [];
}

function normalizeMarkdown(markdown) {
  return `${String(markdown).trim()}\n`;
}

function resolveConfiguredPath(baseDir, configuredPath) {
  const value = String(configuredPath ?? "").trim();
  if (!value) return baseDir;
  if (value === "~/Desktop" || value === "~\\Desktop") return desktopPath();
  if (value === "~") return os.homedir();
  if (value.startsWith("~/") || value.startsWith("~\\")) return path.join(os.homedir(), value.slice(2));
  return path.resolve(baseDir, value);
}

function desktopPath() {
  if (process.platform === "win32") {
    const oneDriveDesktop = path.join(process.env.OneDriveCommercial ?? process.env.OneDrive ?? "", "Desktop");
    if (process.env.OneDriveCommercial || process.env.OneDrive) return oneDriveDesktop;
  }
  return path.join(os.homedir(), "Desktop");
}

async function writeDigestIndex(runtime, now = new Date()) {
  await fs.mkdir(runtime.outputDir, { recursive: true });
  await fs.writeFile(runtime.indexPath, renderDigestIndex(runtime, now), "utf8");
}

function renderDigestOutput(runtime, date, markdown, evidence, status) {
  const title = `Codex Daily Digest - ${date}`;
  const substantiveSessions = evidence.sessions.filter((session) => !isSupportingSession(session));
  const supportingSessionCount = evidence.sessions.length - substantiveSessions.length;
  const commandCount = substantiveSessions.reduce((count, session) => count + session.commands.length, 0);
  const failureCount = evidence.sessions.reduce((count, session) => count + session.failures.length, 0);
  const workspaceRows = workspaceSummary({ ...evidence, sessions: substantiveSessions })
    .map(([cwd, count]) => `<tr><td>${escapeHtml(cwd)}</td><td class="number">${count}</td></tr>`)
    .join("");
  const sessionRows = evidence.sessions
    .map((session) => `<tr>
      <td><div class="session-title">${escapeHtml(session.title ?? session.id)}</div><div class="session-id">${escapeHtml(session.id)}</div></td>
      <td class="workspace-path">${escapeHtml(session.cwd ?? "(unknown)")}</td>
      <td>${escapeHtml(session.updatedAt ?? "unknown")}</td>
      <td class="number">${session.failures.length}</td>
    </tr>`)
    .join("");
  return htmlDocument({
    title,
    body: `<main class="report-shell">
      <header class="report-header">
        <p class="report-kicker">Codex local report</p>
        <div class="report-heading-row">
          <div>
            <h1>${escapeHtml(title)}</h1>
            <p class="report-description">Yesterday's work, evidence-backed. Inferred claims should cite session evidence.</p>
          </div>
          <a class="report-link" href="../${encodeUriPath(path.basename(runtime.indexPath))}">All digests <span aria-hidden="true">→</span></a>
        </div>
      </header>

      <section class="report-metrics" aria-label="Digest summary">
        ${metricCell("Work sessions", substantiveSessions.length)}
        ${metricCell("Supporting", supportingSessionCount)}
        ${metricCell("Checks run", commandCount)}
        ${metricCell("Failures", failureCount)}
      </section>

      <section class="report-layout">
        <article class="report-body">
          ${markdownToHtml(normalizeMarkdown(markdown))}
        </article>
        <aside class="report-aside">
          <section class="report-panel${status.usedFallback ? " report-panel--warning" : ""}">
            <h2>Report state</h2>
            <dl>
              <dt>Summary</dt><dd>${status.inferenceOk ? "Inference" : "Fallback"}</dd>
              <dt>Fallback</dt><dd>${status.usedFallback ? "Used" : "Not used"}</dd>
              <dt>Window</dt><dd>${escapeHtml(formatWindow(evidence.window))}</dd>
            </dl>
          </section>
          <section class="report-panel">
            <h2>Workspaces</h2>
            <table class="compact-table">${workspaceRows || `<tr><td class="muted">No workspace evidence.</td></tr>`}</table>
          </section>
        </aside>
      </section>

      <details class="evidence-section">
        <summary>Evidence appendix <span>Session inventory and trace points</span></summary>
        <div class="table-scroll">
          <table class="evidence-table">
            <thead>
              <tr><th>Session</th><th>Workspace</th><th>Updated</th><th class="number">Failures</th></tr>
            </thead>
            <tbody>${sessionRows || `<tr><td class="muted" colspan="4">No session evidence found for this date.</td></tr>`}</tbody>
          </table>
        </div>
      </details>
    </main>`,
  });
}

function renderDigestIndex(runtime, now) {
  const entries = Object.entries(runtime.state.generatedDigests ?? {})
    .sort(([a], [b]) => compareYmd(b, a))
    .map(([date, digest]) => ({ date, ...digest }));
  const rows = entries
    .map((entry) => `<tr>
      <td><a class="report-link" href="${encodeUriPath(path.relative(runtime.outputDir, entry.outputPath))}">${escapeHtml(entry.date)}</a></td>
      <td class="number">${entry.sessions ?? 0}</td>
      <td>${entry.inferenceOk ? "Inference" : "Fallback"}</td>
      <td>${escapeHtml(entry.generatedAt ?? "")}</td>
    </tr>`)
    .join("");
  return htmlDocument({
    title: "Codex Daily Digest",
    body: `<main class="report-shell">
      <header class="report-header">
        <p class="report-kicker">Codex local reports</p>
        <h1>Codex Daily Digest</h1>
        <p class="report-description">A durable index of the evidence-backed daily briefings saved in <code>${escapeHtml(runtime.config.digestDirName ?? "Codex Daily Digests")}</code>.</p>
      </header>
      <section class="report-metrics" aria-label="Digest index summary">
        ${metricCell("Digests", entries.length)}
        ${metricCell("Sessions", entries.reduce((sum, entry) => sum + (entry.sessions ?? 0), 0))}
        ${metricCell("Generated", formatLocalDateTime(now))}
      </section>
      <section class="evidence-section">
        <div class="section-heading"><h2>Digest files</h2></div>
        <div class="table-scroll">
          <table class="evidence-table">
            <thead>
              <tr><th>Date</th><th class="number">Sessions</th><th>Summary source</th><th>Generated at</th></tr>
            </thead>
            <tbody>${rows || `<tr><td class="muted" colspan="4">No digests generated yet.</td></tr>`}</tbody>
          </table>
        </div>
      </section>
    </main>`,
  });
}

function htmlDocument({ title, body }) {
  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>${escapeHtml(title)}</title>
  <style>${reportStyles()}</style>
</head>
<body>
${body}
</body>
</html>
`;
}

function metricCell(label, value) {
  return `<div class="metric"><div>${escapeHtml(label)}</div><strong>${escapeHtml(String(value))}</strong></div>`;
}

function formatWindow(window) {
  return `${formatLocalDateTime(window.start)} – ${formatLocalDateTime(window.end)}`;
}

function reportStyles() {
  return `
    :root { color-scheme: light; --ink: #172033; --muted: #667085; --rule: #d9dee7; --paper: #ffffff; --wash: #f6f7f9; --accent: #1d4ed8; --warning: #9a6700; }
    * { box-sizing: border-box; }
    body { margin: 0; background: var(--wash); color: var(--ink); font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; line-height: 1.55; }
    .report-shell { width: min(1120px, calc(100% - 32px)); margin: 0 auto; padding: 40px 0 64px; }
    .report-header { border-bottom: 1px solid var(--rule); padding-bottom: 24px; }
    .report-kicker { margin: 0; color: var(--muted); font-size: .72rem; font-weight: 750; letter-spacing: .12em; text-transform: uppercase; }
    h1, h2, h3, p { margin-top: 0; }
    h1 { max-width: 24ch; margin: 8px 0 0; font-size: clamp(2rem, 5vw, 3.25rem); line-height: 1.05; letter-spacing: -.045em; }
    .report-heading-row { display: flex; align-items: end; justify-content: space-between; gap: 24px; }
    .report-description { max-width: 64ch; margin: 14px 0 0; color: var(--muted); font-size: .95rem; }
    .report-link { color: var(--accent); font-weight: 700; text-decoration: none; text-underline-offset: .2em; white-space: nowrap; }
    .report-link:hover { text-decoration: underline; }
    .report-link:focus-visible { outline: 2px solid var(--accent); outline-offset: 4px; border-radius: 2px; }
    .report-metrics { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 0; border-bottom: 1px solid var(--rule); }
    .metric { min-width: 0; padding: 18px 16px 18px 0; border-right: 1px solid var(--rule); }
    .metric + .metric { padding-left: 16px; }
    .metric:last-child { border-right: 0; }
    .metric div { color: var(--muted); font-size: .7rem; font-weight: 750; letter-spacing: .08em; text-transform: uppercase; }
    .metric strong { display: block; margin-top: 4px; overflow-wrap: anywhere; font-size: 1.2rem; font-variant-numeric: tabular-nums; line-height: 1.2; }
    .report-layout { display: grid; grid-template-columns: minmax(0, 1fr) 250px; gap: clamp(32px, 6vw, 72px); padding: 36px 0; }
    .report-body { min-width: 0; max-width: 72ch; }
    .report-body h2 { margin: 34px 0 12px; padding-bottom: 8px; border-bottom: 1px solid var(--rule); font-size: 1.1rem; letter-spacing: -.01em; }
    .report-body h2:first-child { margin-top: 0; }
    .report-body h3 { margin: 24px 0 8px; font-size: 1rem; }
    .report-body p { margin-bottom: 14px; }
    .report-body ul { margin: 0 0 18px; padding-left: 1.25rem; }
    .report-body li { margin: 7px 0; padding-left: .2rem; }
    .report-body code, .report-description code { padding: .12em .35em; border-radius: 4px; background: #edf0f5; font-family: "Cascadia Code", Consolas, monospace; font-size: .88em; }
    .report-body pre { overflow-x: auto; margin: 18px 0; padding: 16px; border-radius: 8px; background: #172033; color: #f8fafc; font-size: .82rem; line-height: 1.5; }
    .report-body pre code { padding: 0; background: transparent; color: inherit; }
    .report-aside { align-self: start; border-left: 1px solid var(--rule); padding-left: 24px; }
    .report-panel + .report-panel { margin-top: 28px; }
    .report-panel h2, .section-heading h2 { margin-bottom: 10px; font-size: .78rem; letter-spacing: .08em; text-transform: uppercase; }
    .report-panel dl { display: grid; grid-template-columns: 78px minmax(0, 1fr); gap: 9px 12px; margin: 0; font-size: .84rem; }
    .report-panel dt { color: var(--muted); }
    .report-panel dd { margin: 0; overflow-wrap: anywhere; }
    .report-panel--warning { padding: 12px; border-left: 3px solid #e0a100; background: #fff8e8; color: #6d4c00; }
    .compact-table, .evidence-table { width: 100%; border-collapse: collapse; font-size: .84rem; }
    .compact-table td { padding: 8px 0; border-bottom: 1px solid var(--rule); overflow-wrap: anywhere; }
    .evidence-section { border-top: 1px solid var(--rule); padding-top: 22px; }
    .evidence-section summary { display: flex; align-items: baseline; justify-content: space-between; gap: 16px; cursor: pointer; font-size: .78rem; font-weight: 750; letter-spacing: .08em; text-transform: uppercase; }
    .evidence-section summary span { color: var(--muted); font-size: .72rem; font-weight: 600; letter-spacing: normal; text-transform: none; }
    .evidence-section[open] summary { margin-bottom: 16px; }
    .section-heading { display: flex; align-items: baseline; justify-content: space-between; gap: 16px; }
    .section-heading p { margin-bottom: 10px; color: var(--muted); font-size: .84rem; }
    .table-scroll { overflow-x: auto; }
    .evidence-table { min-width: 700px; text-align: left; }
    .evidence-table th { padding: 9px 12px 9px 0; border-bottom: 1px solid var(--ink); color: var(--muted); font-size: .7rem; letter-spacing: .08em; text-transform: uppercase; }
    .evidence-table td { padding: 12px 12px 12px 0; border-bottom: 1px solid var(--rule); vertical-align: top; }
    .number { text-align: right; font-variant-numeric: tabular-nums; }
    .session-title { font-weight: 700; }
    .session-id, .workspace-path { color: var(--muted); font-family: "Cascadia Code", Consolas, monospace; font-size: .75rem; overflow-wrap: anywhere; }
    .muted { color: var(--muted); }
    @media (max-width: 760px) {
      .report-shell { width: min(100% - 24px, 1120px); padding-top: 28px; }
      .report-heading-row, .section-heading { display: block; }
      .report-link { display: inline-block; margin-top: 16px; }
      .report-metrics { grid-template-columns: repeat(2, minmax(0, 1fr)); }
      .metric:nth-child(2n) { border-right: 0; }
      .metric:nth-child(3) { border-top: 1px solid var(--rule); padding-left: 0; }
      .metric:nth-child(4) { border-top: 1px solid var(--rule); }
      .report-layout { grid-template-columns: 1fr; gap: 28px; padding: 28px 0; }
      .report-aside { border-left: 0; border-top: 1px solid var(--rule); padding: 24px 0 0; }
    }
  `;
}

function workspaceSummary(evidence) {
  const byCwd = new Map();
  for (const session of evidence.sessions) {
    const key = session.cwd ?? "(unknown cwd)";
    byCwd.set(key, (byCwd.get(key) ?? 0) + 1);
  }
  return [...byCwd.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
}

function markdownToHtml(markdown) {
  const lines = normalizeMarkdown(markdown).split(/\n/);
  const html = [];
  let listOpen = false;
  let codeOpen = false;
  let codeLines = [];
  const closeList = () => {
    if (listOpen) {
      html.push("</ul>");
      listOpen = false;
    }
  };
  for (const line of lines) {
    if (line.startsWith("```")) {
      if (codeOpen) {
        html.push(`<pre><code>${escapeHtml(codeLines.join("\n"))}</code></pre>`);
        codeLines = [];
        codeOpen = false;
      } else {
        closeList();
        codeOpen = true;
      }
      continue;
    }
    if (codeOpen) {
      codeLines.push(line);
      continue;
    }
    if (!line.trim()) {
      closeList();
      continue;
    }
    const heading = /^(#{1,3})\s+(.+)$/.exec(line);
    if (heading) {
      closeList();
      const level = heading[1].length;
      if (level > 1) html.push(`<h${level}>${inlineMarkdown(heading[2])}</h${level}>`);
      continue;
    }
    const item = /^-\s+(.+)$/.exec(line.trim());
    if (item) {
      if (!listOpen) {
        html.push("<ul>");
        listOpen = true;
      }
      html.push(`<li>${inlineMarkdown(item[1])}</li>`);
      continue;
    }
    closeList();
    html.push(`<p>${inlineMarkdown(line)}</p>`);
  }
  closeList();
  if (codeOpen) html.push(`<pre><code>${escapeHtml(codeLines.join("\n"))}</code></pre>`);
  return html.join("\n");
}

function inlineMarkdown(text) {
  return escapeHtml(text)
    .replace(/`([^`]+)`/g, "<code>$1</code>")
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
}

function escapeHtml(value) {
  return String(value ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function encodeUriPath(value) {
  return String(value).split(path.sep).map(encodeURIComponent).join("/");
}

function formatLocalDateTime(date) {
  return new Date(date).toLocaleString();
}

function digestSchedulerOptions(runtime) {
  return schedulerOptionsFromRuntime(runtime, {
    description: "Codex daily digest",
    scheduleTime: runtime.config.scheduleTime,
    linuxUnitName: "codex-daily-digest",
    macLabel: "com.codex.daily-digest",
  });
}

function printResult(value, json = false) {
  if (json) {
    console.log(JSON.stringify(value, null, 2));
    return;
  }
  if (value.results) {
    console.log(`${value.dryRun ? "Dry run" : "Live run"}: ${value.results.length} digest(s)`);
    for (const result of value.results) console.log(`${result.ok ? "OK" : "FAIL"} ${result.date} ${result.outputPath ?? ""}`.trim());
  } else if (value.fallbackMarkdown && value.dryRun) {
    console.log(value.fallbackMarkdown);
  } else {
    console.log(JSON.stringify(value, null, 2));
  }
}

function help() {
  const defaultRuntime = {
    config: DEFAULT_CONFIG,
    scriptPath: __filename,
    configPath: path.join(__dirname, "config.json"),
    codexHome: path.resolve(__dirname, "../.."),
  };
  return `Codex daily digest

Usage:
  node tools/daily-digest/daily-digest.mjs status [--json]
  node tools/daily-digest/daily-digest.mjs run [--dry-run] [--json]
  node tools/daily-digest/daily-digest.mjs generate --date YYYY-MM-DD [--dry-run|--yes] [--json]
  node tools/daily-digest/daily-digest.mjs install-scheduler
  node tools/daily-digest/daily-digest.mjs uninstall-scheduler

Manual generate defaults to dry-run. Pass --yes to write the digest. The scheduled run command is live by design.
Scheduler command: ${schedulerCommand(digestSchedulerOptions(defaultRuntime))}
`;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.command === "help" || args.command === "--help" || args.command === "-h") {
    console.log(help());
    return;
  }
  const runtime = await loadRuntime(args.configPath);
  let result;
  if (args.command === "status") result = await getStatus(runtime);
  else if (args.command === "run") result = await runDue(runtime, undefined, { dryRun: args.dryRun });
  else if (args.command === "generate") result = await generateDigest(runtime, { date: args.date, dryRun: !args.yes || args.dryRun });
  else if (args.command === "install-scheduler") result = await installScheduler(runtime);
  else if (args.command === "uninstall-scheduler") result = await uninstallScheduler(runtime);
  else throw new Error(`Unknown command: ${args.command}`);
  printResult(result, args.json);
  if (result?.ok === false) process.exitCode = result.status || 1;
}

if (process.argv[1] && path.resolve(process.argv[1]) === __filename) {
  main().catch((error) => {
    console.error(error.stack || error.message);
    process.exitCode = 1;
  });
}

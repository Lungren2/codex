import { spawnSync } from "node:child_process";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";

export const UUID_RE = /([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})/i;

export function deepMerge(base, override) {
  const out = { ...base };
  for (const [key, value] of Object.entries(override ?? {})) {
    if (value && typeof value === "object" && !Array.isArray(value) && base[key]) {
      out[key] = deepMerge(base[key], value);
    } else {
      out[key] = value;
    }
  }
  return out;
}

export async function readJson(file, fallback) {
  try {
    return JSON.parse(await fs.readFile(file, "utf8"));
  } catch (error) {
    if (error.code === "ENOENT") return structuredClone(fallback);
    throw new Error(`Failed to read JSON ${file}: ${error.message}`);
  }
}

export async function writeJsonAtomic(file, value) {
  await fs.mkdir(path.dirname(file), { recursive: true });
  const tmp = `${file}.${process.pid}.${Date.now()}.tmp`;
  await fs.writeFile(tmp, `${JSON.stringify(value, null, 2)}\n`, "utf8");
  await fs.rename(tmp, file);
}

export async function createRuntime(options) {
  const {
    configPath,
    defaultConfig,
    defaultState,
    scriptPath,
    lockName,
    defaultConfigPath = path.join(path.dirname(scriptPath), "config.json"),
  } = options;
  const resolvedConfigPath = path.resolve(configPath ?? defaultConfigPath);
  const toolDir = path.dirname(resolvedConfigPath);
  const config = deepMerge(defaultConfig, await readJson(resolvedConfigPath, defaultConfig));
  const codexHome = path.resolve(toolDir, config.codexHome);
  const statePath = path.join(toolDir, "state.json");
  const state = deepMerge(defaultState, await readJson(statePath, defaultState));
  return {
    config,
    configPath: resolvedConfigPath,
    codexHome,
    state,
    statePath,
    toolDir,
    logDir: path.join(toolDir, "logs"),
    lockPath: path.join(toolDir, lockName),
    scriptPath,
  };
}

export function parseScheduleTime(scheduleTime) {
  const match = /^([01]?\d|2[0-3]):([0-5]\d)$/.exec(scheduleTime);
  if (!match) throw new Error(`Invalid schedule time: ${scheduleTime}`);
  return { hour: Number(match[1]), minute: Number(match[2]) };
}

export function scheduledForDate(date, scheduleTime) {
  const { hour, minute } = parseScheduleTime(scheduleTime);
  return new Date(date.getFullYear(), date.getMonth(), date.getDate(), hour, minute, 0, 0);
}

export function addDays(date, days) {
  const next = new Date(date);
  next.setDate(next.getDate() + days);
  return next;
}

export function latestScheduledAtOrBefore(now, scheduleTime) {
  const today = scheduledForDate(now, scheduleTime);
  if (now >= today) return today;
  return addDays(today, -1);
}

export function nextScheduledAfter(lastRunAt, scheduleTime, intervalDays) {
  const lastRun = new Date(lastRunAt);
  const lastWindow = latestScheduledAtOrBefore(lastRun, scheduleTime);
  return addDays(lastWindow, intervalDays);
}

export function getDueStatus(now, lastRunAt, scheduleTime, intervalDays) {
  const latestWindow = latestScheduledAtOrBefore(now, scheduleTime);
  if (!lastRunAt) return { due: true, scheduledAt: latestWindow.toISOString(), nextDueAt: latestWindow.toISOString() };
  const nextDue = nextScheduledAfter(lastRunAt, scheduleTime, intervalDays);
  return {
    due: latestWindow >= nextDue,
    scheduledAt: latestWindow.toISOString(),
    nextDueAt: nextDue.toISOString(),
  };
}

export function ymdFromDate(date) {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

export function dateFromYmd(ymd) {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(ymd);
  if (!match) throw new Error(`Invalid date: ${ymd}`);
  return new Date(Number(match[1]), Number(match[2]) - 1, Number(match[3]), 0, 0, 0, 0);
}

export function previousYmd(ymd) {
  return ymdFromDate(addDays(dateFromYmd(ymd), -1));
}

export function compareYmd(a, b) {
  return a.localeCompare(b);
}

export async function appendLog(runtime, event) {
  await fs.mkdir(runtime.logDir, { recursive: true });
  const day = new Date().toISOString().slice(0, 10);
  const line = `${JSON.stringify({ timestamp: new Date().toISOString(), ...event })}\n`;
  await fs.appendFile(path.join(runtime.logDir, `${day}.log`), line, "utf8");
}

export async function walkFiles(root, predicate = () => true) {
  const files = [];
  let entries;
  try {
    entries = await fs.readdir(root, { withFileTypes: true });
  } catch (error) {
    if (error.code === "ENOENT") return files;
    throw error;
  }
  for (const entry of entries) {
    const fullPath = path.join(root, entry.name);
    if (entry.isDirectory()) files.push(...await walkFiles(fullPath, predicate));
    else if (entry.isFile() && predicate(fullPath)) files.push(fullPath);
  }
  return files;
}

export function sessionIdFromPath(file) {
  return UUID_RE.exec(path.basename(file))?.[1]?.toLowerCase() ?? null;
}

export async function readFirstJsonLine(file) {
  const handle = await fs.open(file, "r");
  try {
    const buffer = Buffer.alloc(128 * 1024);
    const { bytesRead } = await handle.read(buffer, 0, buffer.length, 0);
    const firstLine = buffer.subarray(0, bytesRead).toString("utf8").split(/\r?\n/, 1)[0];
    return firstLine ? JSON.parse(firstLine) : null;
  } catch {
    return null;
  } finally {
    await handle.close();
  }
}

export async function readSessionIndex(codexHome) {
  const indexPath = path.join(codexHome, "session_index.jsonl");
  const sessions = new Map();
  let text;
  try {
    text = await fs.readFile(indexPath, "utf8");
  } catch (error) {
    if (error.code === "ENOENT") return sessions;
    throw error;
  }
  for (const line of text.split(/\r?\n/)) {
    if (!line.trim()) continue;
    try {
      const row = JSON.parse(line);
      if (row.id) {
        sessions.set(row.id.toLowerCase(), {
          id: row.id.toLowerCase(),
          name: row.thread_name ?? null,
          updatedAt: row.updated_at ? new Date(row.updated_at) : null,
          indexed: true,
        });
      }
    } catch {
      // Malformed index rows should not block file-based discovery.
    }
  }
  return sessions;
}

export async function discoverActiveSessionIds(codexHome) {
  const active = new Set();
  const activePath = path.join(codexHome, "process_manager", "chat_processes.json");
  try {
    const rows = JSON.parse(await fs.readFile(activePath, "utf8"));
    for (const row of Array.isArray(rows) ? rows : []) {
      if (row.conversationId) active.add(String(row.conversationId).toLowerCase());
    }
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  if (process.env.CODEX_SESSION_ID) active.add(process.env.CODEX_SESSION_ID.toLowerCase());
  return active;
}

export async function discoverArchivedSessions(codexHome) {
  const archivedDir = path.join(codexHome, "archived_sessions");
  const files = await walkFiles(archivedDir, (file) => file.endsWith(".jsonl"));
  const archived = new Map();
  for (const file of files) {
    const id = sessionIdFromPath(file);
    if (!id) continue;
    const stat = await fs.stat(file);
    archived.set(id, { id, path: file, mtime: stat.mtime, archived: true });
  }
  return archived;
}

export async function discoverSavedSessions(codexHome) {
  const indexed = await readSessionIndex(codexHome);
  const files = await walkFiles(path.join(codexHome, "sessions"), (file) => file.endsWith(".jsonl"));
  for (const file of files) {
    const id = sessionIdFromPath(file);
    if (!id) continue;
    const stat = await fs.stat(file);
    const existing = indexed.get(id) ?? { id, name: null, updatedAt: null, indexed: false };
    let metaTimestamp = null;
    if (!existing.updatedAt) {
      const first = await readFirstJsonLine(file);
      metaTimestamp = first?.payload?.timestamp ? new Date(first.payload.timestamp) : null;
    }
    const updatedAt = maxDate(existing.updatedAt, stat.mtime, metaTimestamp);
    indexed.set(id, { ...existing, id, path: file, mtime: stat.mtime, updatedAt, archived: false });
  }
  return indexed;
}

export function maxDate(...dates) {
  const valid = dates.filter((date) => date instanceof Date && !Number.isNaN(date.valueOf()));
  if (valid.length === 0) return null;
  return new Date(Math.max(...valid.map((date) => date.valueOf())));
}

export function runCommand(command, args, cwd, options = {}) {
  const result = spawnSync(command, args, {
    cwd,
    encoding: "utf8",
    input: options.input,
    timeout: options.timeoutMs,
    windowsHide: true,
  });
  return {
    ok: result.status === 0,
    status: result.status,
    stdout: result.stdout?.trim() ?? "",
    stderr: result.stderr?.trim() ?? "",
    error: result.error?.message ?? null,
  };
}

export async function withLock(runtime, fn) {
  await fs.mkdir(path.dirname(runtime.lockPath), { recursive: true });
  let handle;
  try {
    handle = await fs.open(runtime.lockPath, "wx");
  } catch (error) {
    if (error.code !== "EEXIST") throw error;
    const stat = await fs.stat(runtime.lockPath);
    const staleMs = runtime.config.lockStaleMinutes * 60 * 1000;
    if (Date.now() - stat.mtimeMs <= staleMs) {
      throw new Error(`Another maintenance run is active: ${runtime.lockPath}`);
    }
    await fs.rm(runtime.lockPath, { force: true });
    handle = await fs.open(runtime.lockPath, "wx");
  }
  try {
    await handle.writeFile(JSON.stringify({ pid: process.pid, acquiredAt: new Date().toISOString() }));
    return await fn();
  } finally {
    await handle.close();
    await fs.rm(runtime.lockPath, { force: true });
  }
}

export function schedulerCommand(options) {
  return `"${options.nodeCommand}" "${options.scriptPath}" ${options.runArgs.join(" ")}`;
}

export function schedulerOptionsFromRuntime(runtime, options = {}) {
  return {
    taskName: options.taskName ?? runtime.config.schedulerName,
    description: options.description ?? "Codex maintenance task",
    scheduleTime: options.scheduleTime,
    nodeCommand: options.nodeCommand ?? runtime.config.nodeCommand ?? "node",
    scriptPath: options.scriptPath ?? runtime.scriptPath,
    configPath: options.configPath ?? runtime.configPath,
    codexHome: options.codexHome ?? runtime.codexHome,
    runArgs: options.runArgs ?? ["run", "--config", runtime.configPath],
    linuxUnitName: options.linuxUnitName,
    macLabel: options.macLabel,
  };
}

export function renderWindowsSchedulerCommand(options) {
  const commandArgs = `"${options.scriptPath}" ${options.runArgs.join(" ")}`;
  const ps = [
    `$Action = New-ScheduledTaskAction -Execute ${quotePowerShell(options.nodeCommand)} -Argument ${quotePowerShell(commandArgs)}`,
    `$Trigger = New-ScheduledTaskTrigger -Daily -At ${quotePowerShell(options.scheduleTime)}`,
    "$Settings = New-ScheduledTaskSettingsSet -StartWhenAvailable -MultipleInstances IgnoreNew",
    `Register-ScheduledTask -TaskName ${quotePowerShell(options.taskName)} -Action $Action -Trigger $Trigger -Settings $Settings -Description ${quotePowerShell(options.description)} -Force | Out-Null`,
  ].join("; ");
  return ["powershell.exe", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", ps]];
}

export function renderLinuxSchedulerFiles(options) {
  const unitName = options.linuxUnitName ?? kebabName(options.taskName);
  const serviceName = `${unitName}.service`;
  const timerName = `${unitName}.timer`;
  return {
    serviceName,
    timerName,
    service: `[Unit]
Description=${options.description}

[Service]
Type=oneshot
ExecStart=${options.nodeCommand} ${options.scriptPath} ${options.runArgs.join(" ")}
WorkingDirectory=${options.codexHome}
`,
    timer: `[Unit]
Description=${options.description} timer

[Timer]
OnCalendar=*-*-* ${options.scheduleTime}:00
Persistent=true
Unit=${serviceName}

[Install]
WantedBy=timers.target
`,
  };
}

export function renderMacSchedulerPlist(options) {
  const label = options.macLabel ?? `com.codex.${kebabName(options.taskName)}`;
  const { hour, minute } = parseScheduleTime(options.scheduleTime);
  const args = [options.nodeCommand, options.scriptPath, ...options.runArgs];
  return {
    label,
    plist: `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>${escapeXml(label)}</string>
  <key>ProgramArguments</key>
  <array>
${args.map((arg) => `    <string>${escapeXml(arg)}</string>`).join("\n")}
  </array>
  <key>WorkingDirectory</key><string>${escapeXml(options.codexHome)}</string>
  <key>StartCalendarInterval</key>
  <dict><key>Hour</key><integer>${hour}</integer><key>Minute</key><integer>${minute}</integer></dict>
  <key>RunAtLoad</key><true/>
</dict>
</plist>
`,
  };
}

export async function installScheduler(options) {
  if (process.platform === "win32") return installWindowsScheduler(options);
  if (process.platform === "linux") return installLinuxScheduler(options);
  if (process.platform === "darwin") return installMacScheduler(options);
  throw new Error(`Unsupported platform for scheduler install: ${process.platform}`);
}

export async function uninstallScheduler(options) {
  if (process.platform === "win32") return uninstallWindowsScheduler(options);
  if (process.platform === "linux") return uninstallLinuxScheduler(options);
  if (process.platform === "darwin") return uninstallMacScheduler(options);
  throw new Error(`Unsupported platform for scheduler uninstall: ${process.platform}`);
}

function installWindowsScheduler(options) {
  const [command, args] = renderWindowsSchedulerCommand(options);
  return runCommand(command, args, options.codexHome);
}

function uninstallWindowsScheduler(options) {
  const ps = `Unregister-ScheduledTask -TaskName ${quotePowerShell(options.taskName)} -Confirm:$false -ErrorAction SilentlyContinue`;
  return runCommand("powershell.exe", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", ps], options.codexHome);
}

async function installLinuxScheduler(options) {
  const files = renderLinuxSchedulerFiles(options);
  const dir = path.join(os.homedir(), ".config", "systemd", "user");
  await fs.mkdir(dir, { recursive: true });
  await fs.writeFile(path.join(dir, files.serviceName), files.service, "utf8");
  await fs.writeFile(path.join(dir, files.timerName), files.timer, "utf8");
  const reload = runCommand("systemctl", ["--user", "daemon-reload"], options.codexHome);
  if (!reload.ok) return reload;
  return runCommand("systemctl", ["--user", "enable", "--now", files.timerName], options.codexHome);
}

async function uninstallLinuxScheduler(options) {
  const files = renderLinuxSchedulerFiles(options);
  const stop = runCommand("systemctl", ["--user", "disable", "--now", files.timerName], options.codexHome);
  const dir = path.join(os.homedir(), ".config", "systemd", "user");
  await fs.rm(path.join(dir, files.serviceName), { force: true });
  await fs.rm(path.join(dir, files.timerName), { force: true });
  runCommand("systemctl", ["--user", "daemon-reload"], options.codexHome);
  return stop;
}

async function installMacScheduler(options) {
  const rendered = renderMacSchedulerPlist(options);
  const dir = path.join(os.homedir(), "Library", "LaunchAgents");
  const plistPath = path.join(dir, `${rendered.label}.plist`);
  await fs.mkdir(dir, { recursive: true });
  await fs.writeFile(plistPath, rendered.plist, "utf8");
  return runCommand("launchctl", ["load", "-w", plistPath], options.codexHome);
}

async function uninstallMacScheduler(options) {
  const rendered = renderMacSchedulerPlist(options);
  const plistPath = path.join(os.homedir(), "Library", "LaunchAgents", `${rendered.label}.plist`);
  const unload = runCommand("launchctl", ["unload", "-w", plistPath], options.codexHome);
  await fs.rm(plistPath, { force: true });
  return unload;
}

function quotePowerShell(value) {
  return `'${String(value).replaceAll("'", "''")}'`;
}

function kebabName(value) {
  return String(value)
    .replace(/([a-z0-9])([A-Z])/g, "$1-$2")
    .replace(/[^a-zA-Z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .toLowerCase();
}

function escapeXml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&apos;");
}

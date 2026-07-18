import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  createRuntime,
  discoverArchivedSessions,
  discoverSavedSessions,
  getDueStatus,
  renderLinuxSchedulerFiles,
  renderMacSchedulerPlist,
  renderWindowsSchedulerCommand,
  schedulerOptionsFromRuntime,
  withLock,
  writeJsonAtomic,
} from "./codex-maintenance.mjs";

async function makeFixture() {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "codex-maintenance-"));
  const toolDir = path.join(root, "tools", "sample");
  const codexHome = path.join(root, "codex-home");
  await fs.mkdir(path.join(codexHome, "sessions", "2026", "07", "01"), { recursive: true });
  await fs.mkdir(path.join(codexHome, "archived_sessions"), { recursive: true });
  await fs.mkdir(toolDir, { recursive: true });
  return { root, toolDir, codexHome, configPath: path.join(toolDir, "config.json") };
}

test("createRuntime merges config and state defaults", async () => {
  const fixture = await makeFixture();
  await fs.writeFile(fixture.configPath, JSON.stringify({ codexHome: fixture.codexHome, nested: { b: 2 } }));
  await fs.writeFile(path.join(fixture.toolDir, "state.json"), JSON.stringify({ seen: { one: true } }));

  const runtime = await createRuntime({
    configPath: fixture.configPath,
    defaultConfig: { codexHome: "../..", nested: { a: 1, b: 1 } },
    defaultState: { seen: {}, lastRunAt: null },
    scriptPath: path.join(fixture.toolDir, "sample.mjs"),
    lockName: "sample.lock",
  });

  assert.equal(runtime.codexHome, fixture.codexHome);
  assert.deepEqual(runtime.config.nested, { a: 1, b: 2 });
  assert.deepEqual(runtime.state, { seen: { one: true }, lastRunAt: null });
});

test("writeJsonAtomic persists formatted JSON", async () => {
  const fixture = await makeFixture();
  const file = path.join(fixture.toolDir, "nested", "state.json");
  await writeJsonAtomic(file, { ok: true });
  assert.match(await fs.readFile(file, "utf8"), /"ok": true/);
});

test("due status catches missed windows once", () => {
  const now = new Date(2026, 6, 4, 9, 0, 0);
  const lastRunAt = new Date(2026, 6, 1, 8, 5, 0).toISOString();
  assert.equal(getDueStatus(now, lastRunAt, "08:00", 1).due, true);
  assert.equal(getDueStatus(now, now.toISOString(), "08:00", 1).due, false);
});

test("withLock replaces stale locks", async () => {
  const fixture = await makeFixture();
  const runtime = {
    lockPath: path.join(fixture.toolDir, "sample.lock"),
    config: { lockStaleMinutes: 1 },
  };
  await fs.writeFile(runtime.lockPath, "stale");
  const old = new Date(Date.now() - 10 * 60 * 1000);
  await fs.utimes(runtime.lockPath, old, old);

  const result = await withLock(runtime, async () => "ok");
  assert.equal(result, "ok");
  await assert.rejects(fs.stat(runtime.lockPath), /ENOENT/);
});

test("session discovery reads index, session files, and archived files", async () => {
  const fixture = await makeFixture();
  const id = "11111111-1111-4111-8111-111111111111";
  const archivedId = "22222222-2222-4222-8222-222222222222";
  const sessionPath = path.join(fixture.codexHome, "sessions", "2026", "07", "01", `rollout-2026-07-01T08-00-00-${id}.jsonl`);
  await fs.writeFile(sessionPath, `${JSON.stringify({ type: "session_meta", payload: { id, timestamp: "2026-07-01T08:00:00.000Z" } })}\n`);
  await fs.writeFile(path.join(fixture.codexHome, "session_index.jsonl"), `${JSON.stringify({ id, thread_name: "Thread", updated_at: "2026-07-01T09:00:00.000Z" })}\n`);
  await fs.writeFile(path.join(fixture.codexHome, "archived_sessions", `rollout-2026-07-01T08-00-00-${archivedId}.jsonl`), "{}\n");

  const saved = await discoverSavedSessions(fixture.codexHome);
  const archived = await discoverArchivedSessions(fixture.codexHome);
  assert.equal(saved.get(id).name, "Thread");
  assert.equal(saved.get(id).path, sessionPath);
  assert.equal(archived.has(archivedId), true);
});

test("scheduler renderers include task name, schedule, and script args", () => {
  const runtime = {
    config: { schedulerName: "CodexDailyDigest", nodeCommand: "node" },
    scriptPath: "C:\\Users\\EthanO\\.codex\\tools\\daily-digest\\daily-digest.mjs",
    configPath: "C:\\Users\\EthanO\\.codex\\tools\\daily-digest\\config.json",
    codexHome: "C:\\Users\\EthanO\\.codex",
  };
  const options = schedulerOptionsFromRuntime(runtime, {
    description: "Codex daily digest",
    scheduleTime: "08:30",
    linuxUnitName: "codex-daily-digest",
    macLabel: "com.codex.daily-digest",
  });

  const windows = renderWindowsSchedulerCommand(options);
  const linux = renderLinuxSchedulerFiles(options);
  const mac = renderMacSchedulerPlist(options);
  assert.equal(windows[0], "powershell.exe");
  assert.match(windows[1].join(" "), /CodexDailyDigest/);
  assert.match(linux.timer, /OnCalendar=\*-\*-\* 08:30:00/);
  assert.match(mac.plist, /com.codex.daily-digest/);
});

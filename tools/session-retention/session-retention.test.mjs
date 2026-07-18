import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  archiveEligible,
  deleteEligible,
  getArchiveCandidates,
  getDeleteCandidates,
  getDueStatus,
  loadRuntime,
} from "./session-retention.mjs";

async function makeFixture() {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "codex-retention-"));
  const toolDir = path.join(root, "tools", "session-retention");
  const codexHome = path.join(root, "codex-home");
  await fs.mkdir(toolDir, { recursive: true });
  await fs.mkdir(path.join(codexHome, "sessions", "2026", "07", "01"), { recursive: true });
  await fs.mkdir(path.join(codexHome, "archived_sessions"), { recursive: true });
  await fs.mkdir(path.join(codexHome, "process_manager"), { recursive: true });
  await fs.writeFile(path.join(toolDir, "config.json"), JSON.stringify({
    codexHome,
    archive: { scheduleTime: "08:00", intervalDays: 1, inactiveHours: 24 },
    delete: { scheduleTime: "08:00", intervalDays: 1, retentionDays: 2 },
  }, null, 2));
  await fs.writeFile(path.join(toolDir, "state.json"), JSON.stringify({
    lastArchiveRunAt: null,
    lastDeleteRunAt: null,
    archivedSessions: {},
    deletedSessions: {},
  }, null, 2));
  return { root, toolDir, codexHome, configPath: path.join(toolDir, "config.json") };
}

async function writeSession(codexHome, id, updatedAt, indexed = true) {
  const file = path.join(codexHome, "sessions", "2026", "07", "01", `rollout-2026-07-01T08-00-00-${id}.jsonl`);
  const meta = { timestamp: updatedAt, type: "session_meta", payload: { id, timestamp: updatedAt } };
  await fs.writeFile(file, `${JSON.stringify(meta)}\n`);
  await fs.utimes(file, new Date(updatedAt), new Date(updatedAt));
  if (indexed) {
    await fs.appendFile(path.join(codexHome, "session_index.jsonl"), `${JSON.stringify({ id, thread_name: id, updated_at: updatedAt })}\n`);
  }
  return file;
}

async function writeArchived(codexHome, id) {
  const file = path.join(codexHome, "archived_sessions", `rollout-2026-07-01T08-00-00-${id}.jsonl`);
  await fs.writeFile(file, `${JSON.stringify({ type: "session_meta", payload: { id } })}\n`);
  return file;
}

test("due status runs once for missed windows", () => {
  const now = new Date(2026, 6, 4, 9, 0, 0);
  const lastRunAt = new Date(2026, 6, 1, 8, 5, 0).toISOString();
  const due = getDueStatus(now, lastRunAt, "08:00", 1);
  assert.equal(due.due, true);

  const afterRun = getDueStatus(now, now.toISOString(), "08:00", 1);
  assert.equal(afterRun.due, false);
});

test("archive candidates skip active, archived, and fresh sessions", async () => {
  const fixture = await makeFixture();
  const runtime = await loadRuntime(fixture.configPath);
  const oldId = "11111111-1111-4111-8111-111111111111";
  const freshId = "22222222-2222-4222-8222-222222222222";
  const activeId = "33333333-3333-4333-8333-333333333333";
  const alreadyArchivedId = "44444444-4444-4444-8444-444444444444";
  await writeSession(fixture.codexHome, oldId, "2026-07-01T06:00:00.000Z");
  await writeSession(fixture.codexHome, freshId, "2026-07-02T07:00:00.000Z");
  await writeSession(fixture.codexHome, activeId, "2026-07-01T06:00:00.000Z");
  await writeSession(fixture.codexHome, alreadyArchivedId, "2026-07-01T06:00:00.000Z");
  await writeArchived(fixture.codexHome, alreadyArchivedId);
  await fs.writeFile(path.join(fixture.codexHome, "process_manager", "chat_processes.json"), JSON.stringify([{ conversationId: activeId }]));

  const candidates = await getArchiveCandidates(runtime, new Date("2026-07-02T08:30:00.000Z"));
  assert.deepEqual(candidates.map((candidate) => candidate.id), [oldId]);
});

test("delete candidates require service archivedAt timestamp and retention age", async () => {
  const fixture = await makeFixture();
  const runtime = await loadRuntime(fixture.configPath);
  const oldArchived = "55555555-5555-4555-8555-555555555555";
  const newArchived = "66666666-6666-4666-8666-666666666666";
  const untracked = "77777777-7777-4777-8777-777777777777";
  await writeArchived(fixture.codexHome, oldArchived);
  await writeArchived(fixture.codexHome, newArchived);
  await writeArchived(fixture.codexHome, untracked);
  runtime.state.archivedSessions[oldArchived] = { archivedAt: "2026-06-29T08:00:00.000Z" };
  runtime.state.archivedSessions[newArchived] = { archivedAt: "2026-07-01T18:00:00.000Z" };

  const candidates = await getDeleteCandidates(runtime, new Date("2026-07-02T08:30:00.000Z"));
  assert.deepEqual(candidates.map((candidate) => candidate.id), [oldArchived]);
});

test("live archive and delete call codex CLI with exact arguments", async () => {
  const fixture = await makeFixture();
  const archiveId = "88888888-8888-4888-8888-888888888888";
  const deleteId = "99999999-9999-4999-8999-999999999999";
  await writeSession(fixture.codexHome, archiveId, "2026-07-01T06:00:00.000Z");
  await writeArchived(fixture.codexHome, deleteId);
  const callsPath = path.join(fixture.root, "codex-calls.jsonl");
  const stubPath = path.join(fixture.root, "fake-codex.mjs");
  await fs.writeFile(stubPath, [
    "import fs from 'node:fs';",
    `fs.appendFileSync(${JSON.stringify(callsPath)}, JSON.stringify(process.argv.slice(2)) + '\\n');`,
  ].join("\n"));

  const runtime = await loadRuntime(fixture.configPath);
  runtime.config.codexCommand = process.execPath;
  runtime.config.codexCommandArgs = [stubPath];
  runtime.state.archivedSessions[deleteId] = { archivedAt: "2026-06-29T08:00:00.000Z" };
  const progress = [];

  await archiveEligible(runtime, {
    dryRun: false,
    now: new Date("2026-07-02T08:30:00.000Z"),
    onProgress: (event) => progress.push(event),
  });
  await deleteEligible(runtime, {
    dryRun: false,
    now: new Date("2026-07-02T08:30:00.000Z"),
    onProgress: (event) => progress.push(event),
  });

  const calls = (await fs.readFile(callsPath, "utf8")).trim().split(/\r?\n/).map((line) => JSON.parse(line));
  assert.deepEqual(calls, [
    ["archive", archiveId],
    ["delete", "--force", deleteId],
  ]);
  assert.deepEqual(progress.map((event) => [event.current, event.total, event.record.action, event.record.id]), [
    [1, 1, "archive", archiveId],
    [1, 1, "delete", deleteId],
  ]);
  assert.equal(runtime.state.archivedSessions[archiveId].archivedAt, "2026-07-02T08:30:00.000Z");
  assert.equal(runtime.state.deletedSessions[deleteId].deletedAt, "2026-07-02T08:30:00.000Z");
});

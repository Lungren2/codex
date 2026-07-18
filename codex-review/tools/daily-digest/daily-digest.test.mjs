import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  buildEvidenceBundle,
  generateDigest,
  getDueDigestDates,
  loadRuntime,
} from "./daily-digest.mjs";

async function makeFixture() {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "codex-digest-"));
  const toolDir = path.join(root, "tools", "daily-digest");
  const codexHome = path.join(root, "codex-home");
  await fs.mkdir(path.join(toolDir, "digests"), { recursive: true });
  await fs.mkdir(path.join(toolDir, "tmp"), { recursive: true });
  await fs.mkdir(path.join(codexHome, "sessions", "2026", "07", "01"), { recursive: true });
  await fs.mkdir(path.join(codexHome, "process_manager"), { recursive: true });
  await fs.writeFile(path.join(toolDir, "config.json"), JSON.stringify({
    codexHome,
    outputDir: "digests",
    tmpDir: "tmp",
    scheduleTime: "08:30",
    maxCatchUpDays: 3,
    includeRetentionLogs: false,
  }, null, 2));
  await fs.writeFile(path.join(toolDir, "state.json"), JSON.stringify({
    lastRunAt: null,
    generatedDigests: {},
    lastFailure: null,
  }, null, 2));
  return { root, toolDir, codexHome, configPath: path.join(toolDir, "config.json") };
}

async function writeSession(codexHome, id, updatedAt, options = {}) {
  const cwd = options.cwd ?? "D:\\dev\\example";
  const userPrompt = options.userPrompt ?? "Fix tools/daily-digest/daily-digest.mjs";
  const file = path.join(codexHome, "sessions", "2026", "07", "01", `rollout-2026-07-01T08-00-00-${id}.jsonl`);
  const lines = [
    { type: "session_meta", payload: { id, timestamp: updatedAt, cwd, cli_version: "test", base_instructions: "DO NOT INCLUDE" } },
    { type: "response_item", payload: { role: "user", content: [{ text: userPrompt }] } },
    { type: "response_item", payload: { role: "assistant", content: [{ text: "Ran node --test tools/daily-digest/daily-digest.test.mjs" }] } },
    { type: "event_msg", payload: { message: "failed command status 1 in tools/example.test.mjs" } },
  ];
  await fs.writeFile(file, `${lines.map((line) => JSON.stringify(line)).join("\n")}\n`);
  await fs.utimes(file, new Date(updatedAt), new Date(updatedAt));
  await fs.writeFile(path.join(codexHome, "session_index.jsonl"), `${JSON.stringify({ id, thread_name: "Digest thread", updated_at: updatedAt })}\n`);
  return file;
}

test("due digest dates catch up missing days within cap", async () => {
  const fixture = await makeFixture();
  const runtime = await loadRuntime(fixture.configPath);
  const due = getDueDigestDates(runtime, new Date(2026, 6, 4, 9, 0, 0));
  assert.deepEqual(due, ["2026-07-01", "2026-07-02", "2026-07-03"]);
});

test("evidence extraction skips base instructions and captures useful events", async () => {
  const fixture = await makeFixture();
  const id = "11111111-1111-4111-8111-111111111111";
  await writeSession(fixture.codexHome, id, "2026-07-01T08:15:00.000Z");
  const runtime = await loadRuntime(fixture.configPath);

  const evidence = await buildEvidenceBundle(runtime, "2026-07-01");
  const serialized = JSON.stringify(evidence);
  assert.equal(evidence.sessions.length, 1);
  assert.equal(evidence.sessions[0].cwd, "D:\\dev\\example");
  assert.match(serialized, /daily-digest\.mjs/);
  assert.doesNotMatch(serialized, /DO NOT INCLUDE/);
  assert.equal(evidence.sessions[0].failures.length, 1);
});

test("evidence extraction keeps real tool commands but excludes assistant narration", async () => {
  const fixture = await makeFixture();
  const id = "66666666-6666-4666-8666-666666666666";
  const file = await writeSession(fixture.codexHome, id, "2026-07-01T08:15:00.000Z");
  await fs.appendFile(file, `${JSON.stringify({ type: "response_item", payload: { type: "custom_tool_call", name: "exec", input: "tools.exec_command({\\\"cmd\\\":\\\"node --test daily-digest.test.mjs\\\"})" } })}\n`);
  await fs.utimes(file, new Date("2026-07-01T08:15:00.000Z"), new Date("2026-07-01T08:15:00.000Z"));
  const runtime = await loadRuntime(fixture.configPath);

  const evidence = await buildEvidenceBundle(runtime, "2026-07-01");
  assert.deepEqual(evidence.sessions[0].commands.map((command) => command.command), ["node --test daily-digest.test.mjs"]);
  assert.equal(evidence.sessions[0].failures.length, 1);
});

test("dry-run generate returns fallback markdown without writing report", async () => {
  const fixture = await makeFixture();
  const id = "22222222-2222-4222-8222-222222222222";
  await writeSession(fixture.codexHome, id, "2026-07-01T08:15:00.000Z");
  const runtime = await loadRuntime(fixture.configPath);

  const result = await generateDigest(runtime, { date: "2026-07-01", dryRun: true, now: new Date("2026-07-02T08:30:00.000Z") });
  assert.equal(result.dryRun, true);
  assert.match(result.fallbackMarkdown, /Codex Daily Digest - 2026-07-01/);
  await assert.rejects(fs.stat(result.outputPath), /ENOENT/);
});

test("fallback digest demotes temporary documentation-agent sessions", async () => {
  const fixture = await makeFixture();
  await writeSession(fixture.codexHome, "55555555-5555-4555-8555-555555555555", "2026-07-01T08:15:00.000Z", {
    cwd: "C:\\Users\\EthanO\\AppData\\Local\\Temp\\tailwind-doc-agent-example",
    userPrompt: "You are a Tailwind CSS v4 documentation subagent for a frontend design workflow.",
  });
  const runtime = await loadRuntime(fixture.configPath);
  const result = await generateDigest(runtime, { date: "2026-07-01", dryRun: true, now: new Date("2026-07-02T08:30:00.000Z") });

  assert.match(result.fallbackMarkdown, /No substantive implementation session was captured/);
  assert.match(result.fallbackMarkdown, /Supporting or temporary sessions: 1/);
  assert.doesNotMatch(result.fallbackMarkdown, /Latest user prompt/);
});

test("live generate invokes codex exec with exact arguments and writes summary", async () => {
  const fixture = await makeFixture();
  const id = "33333333-3333-4333-8333-333333333333";
  await writeSession(fixture.codexHome, id, "2026-07-01T08:15:00.000Z");
  const callsPath = path.join(fixture.root, "codex-calls.jsonl");
  const stubPath = path.join(fixture.root, "fake-codex.mjs");
  await fs.writeFile(stubPath, [
    "import fs from 'node:fs';",
    "const args = process.argv.slice(2);",
    `fs.appendFileSync(${JSON.stringify(callsPath)}, JSON.stringify(args) + '\\n');`,
    "const output = args[args.indexOf('-o') + 1];",
    "fs.writeFileSync(output, '# Inferred digest\\n\\n## Summary\\n\\n- Evidence-backed summary\\n');",
  ].join("\n"));

  const runtime = await loadRuntime(fixture.configPath);
  runtime.config.codexCommand = process.execPath;
  runtime.config.codexCommandArgs = [stubPath];

  const result = await generateDigest(runtime, { date: "2026-07-01", dryRun: false, now: new Date("2026-07-02T08:30:00.000Z") });
  const calls = (await fs.readFile(callsPath, "utf8")).trim().split(/\r?\n/).map((line) => JSON.parse(line));
  assert.deepEqual(calls[0].slice(0, 10), [
    "exec",
    "--ephemeral",
    "--skip-git-repo-check",
    "--sandbox",
    "read-only",
    "-C",
    fixture.codexHome,
    "-o",
    path.join(fixture.toolDir, "tmp", "2026-07-01.summary.md"),
    "-",
  ]);
  assert.equal(calls[0].at(-1), "-");
  assert.equal(result.inferenceOk, true);
  assert.equal(path.basename(result.outputPath), "2026-07-01.html");
  assert.equal(path.basename(path.dirname(result.outputPath)), "Codex Daily Digests");
  const digestHtml = await fs.readFile(result.outputPath, "utf8");
  assert.match(digestHtml, /class="report-shell"/);
  assert.match(digestHtml, /\.report-body h2/);
  assert.match(digestHtml, /@media \(max-width: 760px\)/);
  assert.doesNotMatch(digestHtml, /cdn\.tailwindcss\.com/);
  assert.doesNotMatch(digestHtml, /\bprose\b/);
  assert.match(digestHtml, /Evidence-backed summary/);
  assert.match(digestHtml, /Evidence appendix/);
  assert.match(digestHtml, /Work sessions/);
  assert.doesNotMatch(digestHtml, /<h1>Inferred digest<\/h1>/);
  const indexHtml = await fs.readFile(result.indexPath, "utf8");
  assert.match(indexHtml, /Codex%20Daily%20Digests\/2026-07-01\.html/);
});

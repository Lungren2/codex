import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import process from "node:process";

const codex = process.platform === "win32" ? "codex.exe" : "codex";
const child = spawn(codex, ["app-server", "--stdio", "--disable", "hooks"], {
  cwd: process.cwd(),
  stdio: ["pipe", "pipe", "inherit"],
});
const pending = new Map();
let nextId = 1;
createInterface({ input: child.stdout }).on("line", (line) => {
  const message = JSON.parse(line);
  if (message.id !== undefined && pending.has(message.id)) {
    const { resolve, reject } = pending.get(message.id);
    pending.delete(message.id);
    if (message.error) reject(new Error(message.error.message));
    else resolve(message.result);
  }
});

try {
  await request("initialize", {
    clientInfo: { name: "codex_review_live_spike", title: "Codex Review Live Spike", version: "0.1.0" },
    capabilities: { experimentalApi: true, optOutNotificationMethods: ["item/agentMessage/delta"] },
  });
  notify("initialized");
  const started = await request("thread/start", {
    cwd: process.cwd(),
    approvalPolicy: "never",
    sandbox: "read-only",
    ephemeral: true,
    serviceName: "codex-review-live-spike",
  });
  if (started.thread.ephemeral !== true || started.thread.path !== null) {
    throw new Error("thread/start did not return a pathless ephemeral thread");
  }
  const listed = await request("thread/list", { limit: 100 });
  if (listed.data.some((thread) => thread.id === started.thread.id)) {
    throw new Error("ephemeral synthesis thread leaked into thread/list");
  }
  await request("thread/unsubscribe", { threadId: started.thread.id });
  process.stdout.write(`Verified pathless, unlisted ephemeral thread ${started.thread.id}\n`);
} finally {
  child.stdin.end();
  child.kill();
}

function request(method, params) {
  const id = nextId++;
  child.stdin.write(`${JSON.stringify({ id, method, params })}\n`);
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      pending.delete(id);
      reject(new Error(`${method} timed out`));
    }, 30_000);
    pending.set(id, {
      resolve: (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      reject: (error) => {
        clearTimeout(timer);
        reject(error);
      },
    });
  });
}

function notify(method) {
  child.stdin.write(`${JSON.stringify({ method })}\n`);
}

import { spawn } from "node:child_process";
import { mkdir } from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { chromium } from "playwright";

const root = path.resolve(import.meta.dirname, "..");
const screenshotDir = path.join(root, "screenshots");
const port = process.env.CODEX_REVIEW_CDP_PORT ?? "9222";
const endpoint = `http://127.0.0.1:${port}`;
const command = process.platform === "win32" ? process.env.ComSpec ?? "cmd.exe" : "pnpm";
const spawnArgs = process.platform === "win32" ? ["/d", "/s", "/c", "pnpm tauri dev"] : ["tauri", "dev"];

await mkdir(screenshotDir, { recursive: true });
const child = spawn(command, spawnArgs, {
  cwd: root,
  stdio: "inherit",
  env: {
    ...process.env,
    CARGO_TARGET_DIR:
      process.env.CARGO_TARGET_DIR ??
      path.join(process.env.USERPROFILE ?? root, ".cache", "codex-review-target"),
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
  },
});

let browser;
let page;
try {
  await waitForEndpoint(`${endpoint}/json/list`, 180_000);
  browser = await chromium.connectOverCDP(endpoint);
  page = browser.contexts()[0]?.pages()[0];
  if (!page) throw new Error("Tauri WebView did not expose a page");
  await page.goto("http://localhost:1420/?demo=1");
  await page.locator(".compact-repository").getByText("codex-review", { exact: true }).waitFor();
  await page.waitForFunction(() =>
    [...document.querySelectorAll("diffs-container")].some((node) => node.shadowRoot?.textContent?.includes("artifactId")),
    undefined,
    { timeout: 30_000 },
  );

  await capture(page, "review-wide", { width: 1500, height: 940 });
  const wideLayout = await page.evaluate(() => {
    const rail = document.querySelector(".compact-rail")?.getBoundingClientRect();
    const diff = document.querySelector(".compact-diff")?.getBoundingClientRect();
    const bar = document.querySelector(".compact-bar")?.getBoundingClientRect();
    return {
      barHeight: bar?.height,
      rail: rail && { x: rail.x, width: rail.width },
      diff: diff && { x: diff.x, width: diff.width },
      overflow: document.documentElement.scrollWidth - window.innerWidth,
      headings: document.querySelectorAll("h1,h2,h3").length,
      segmentedControls: document.querySelectorAll(".segmented-control").length,
    };
  });
  if (wideLayout.overflow > 1 || wideLayout.rail?.x !== 0 || (wideLayout.diff?.x ?? 0) < 200 || wideLayout.headings !== 0 || wideLayout.segmentedControls !== 0) {
    throw new Error(`Compact diff layout is invalid: ${JSON.stringify(wideLayout)}`);
  }
  await capture(page, "review-narrow", { width: 980, height: 800 });
  await capture(page, "review-mobile", { width: 390, height: 800 });

  for (const route of ["utilities", "skills"]) {
    await page.goto(`http://localhost:1420/${route}`);
    await page.locator(`.${route}-shell`).waitFor();
    await capture(page, `${route}-wide`, { width: 1500, height: 940 });
    await capture(page, `${route}-mobile`, { width: 390, height: 800 });
    const shell = await page.evaluate((name) => {
      const root = document.querySelector(`.${name}-shell`);
      const bar = root?.firstElementChild?.getBoundingClientRect();
      return {
        height: root?.getBoundingClientRect().height,
        barHeight: bar?.height,
        overflow: document.documentElement.scrollWidth - window.innerWidth,
      };
    }, route);
    if (shell.height !== 800 || Math.abs((shell.barHeight ?? 0) - (wideLayout.barHeight ?? 0)) > 0.5 || shell.overflow > 1) {
      throw new Error(`${route} shell is inconsistent: ${JSON.stringify(shell)}`);
    }
  }
  process.stdout.write(`Verified Tauri WebView at ${endpoint}\n`);
} finally {
  await page?.evaluate(() => window.__TAURI_INTERNALS__?.invoke("exit_application")).catch(() => undefined);
  await browser?.close().catch(() => undefined);
  child.kill();
}

async function capture(page, name, viewport) {
  await page.setViewportSize(viewport);
  await page.waitForTimeout(700);
  await page.screenshot({ path: path.join(screenshotDir, `${name}.png`), fullPage: false });
}

async function waitForEndpoint(url, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(`Tauri dev exited with code ${child.exitCode} before CDP became ready`);
    }
    try {
      const response = await fetch(url);
      if (response.ok) return;
    } catch {
      // The first Rust build can take a while; keep polling the WebView endpoint.
    }
    await new Promise((resolve) => setTimeout(resolve, 1_000));
  }
  throw new Error(`Timed out waiting for ${url}`);
}

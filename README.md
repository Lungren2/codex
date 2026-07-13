# Codex Review

Codex Review is a private, Windows-first companion for reviewing Codex changes as decisions rather than a flat list of files. Git remains authoritative; model summaries are bounded annotations linked to exact frozen hunks. Its primary surface is a two-pane review: the active file's explanation and scoped feedback stay fixed on the left while a continuously scrolling `@pierre/diffs` review occupies the right.

## Architecture

```text
Codex desktop turn completes
  -> shared notify command invokes `codex-review ingest`
  -> previous notify command is chained unchanged
  -> Git branch/worktree patch is frozen locally
  -> SQLite queues an ephemeral read-only synthesis turn
  -> the Tauri workbench presents decisions, evidence, validation, and revisions
```

The app stores metadata in `%LOCALAPPDATA%\codex-review\review.sqlite3` and content-addressed patches below `%LOCALAPPDATA%\codex-review\patches`. It does not upload review artifacts to a separate service.

## Development

```powershell
pnpm install
pnpm build
pnpm test
$env:CARGO_TARGET_DIR="$env:USERPROFILE\.cache\codex-review-target"
cargo test --manifest-path src-tauri/Cargo.toml
pnpm desktop:dev
```

For the actual WebView2 acceptance flow:

```powershell
pnpm desktop:playwright
```

The script connects Playwright to the Tauri WebView over a loopback CDP port and writes wide, narrow, and mobile-sized screenshots under `screenshots/`.

## Notify installation

After building or installing the executable:

```powershell
codex-review.exe install-notify
codex-review.exe uninstall-notify
```

Installation edits only the top-level `notify` value in `~/.codex/config.toml`, records the exact prior argv locally, and chains that command after capture. Uninstall restores the prior argv only if the configured command is still owned by Codex Review; it refuses to overwrite later user changes. A `.codex-review.bak` copy is written beside the config before each edit.

## Safety boundaries

- The authoritative patch is the branch/worktree delta from the configured override, upstream branch, or `origin/HEAD` merge base.
- Patch bodies are local content-addressed files. Secret-like hunks are retained locally but redacted from synthesis prompts.
- Synthesis uses an ephemeral app-server thread, `read-only` sandbox, `never` approvals, hooks disabled, strict structured output, and fail-closed server requests.
- A standalone app-server cannot safely determine whether Codex desktop currently owns an active rollout. Feedback is therefore saved locally, copied as a scoped prompt, and handed to the desktop app for the user to paste into the originating task. Direct cross-process `thread/resume` is intentionally disabled.
- The Complete diff view is always available and model grouping cannot hide risk-sensitive or unassigned hunks.
- Review links are local artifact identities in v1. Codex desktop emits its final response before the fire-and-forget notify hook runs, so rewriting that already-rendered response with a review URL would require an upstream protocol hook and is intentionally outside this companion-only implementation.

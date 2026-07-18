# Codex Review source

- This repository owns the companion application: notify ingestion, frozen patches, synthesis, persistence, scoped feedback, and the Tauri UI.
- Git is authoritative; model summaries may annotate but never conceal or replace captured changes.
- Runtime data belongs under `%LOCALAPPDATA%\codex-review`; the installed executable belongs under `%LOCALAPPDATA%\Codex Review\bin`. Do not write runtime state into this source tree.
- Reusable Codex protocol or app-server changes belong in `D:\dev\codex` and require explicit scope expansion.
- Use the paths in `D:\dev\codex-dev\ecosystem.json`; do not duplicate machine paths in new scripts.
- Validate frontend changes with `pnpm build`, `pnpm test -- --run`, and `pnpm desktop:playwright`. Validate Rust changes with `cargo fmt --check` and `cargo test` using the configured external target directory.

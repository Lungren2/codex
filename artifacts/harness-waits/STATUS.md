# Automatic inference suspension

User request: implement harness scheduling, rather than a skill/poll recipe.
Any foreground command exceeding a configurable latency threshold must keep
inference suspended until it resolves. Source authority: D:/dev/codex; isolated
development target/home from D:/dev/codex-dev/ecosystem.json. Preserve existing
uncommitted prompt-profile/config changes. No commit/push/deployment authorized.

Implemented in source: optional `tool_inference_pause_threshold_ms` config
(unset/0 retains upstream behavior). Native foreground exec, including PTYs, remains
awaited after the initial threshold, with bounded output and live UI streaming.
Code-mode exec consumes elapsed cell yields inside the harness, preserving
bounded output until terminal resolution. Explicit zero-yield calls opt into
background/interactive control. No Rust-specific or PTY-based intent detection.
Direct synchronous tools already await resolution. Cancellation remains owned
by the existing tool runtime. This does not pause a remote request mid-stream;
it prevents subsequent inference while a pending tool future is unresolved.

Added integration tests asserting mock model-request count stays at one while
an owned tool/command is held, then resumes exactly once on resolution. Include
success, failure, cancellation, explicit background control, bounded accumulated
output and live command output. Completed command: `just test --cargo-profile dev-small -p
codex-core --test inference_pause`, CARGO_TARGET_DIR from the
ecosystem development target, one Cargo job, incremental disabled. Initial cell 106/native handle 33365
finished with three E0308 errors caused by shadowing response content with the
prefix. Corrected the merge to name response_items distinctly. Exact initial
output is retained at C:/Users/EthanO/.codex/exports/thread-errors/analysis/usage-patterns/harness-first-compile.log.
The aggregate retry compiled the harness library, then failed on unrelated
pre-existing duplicate helpers/stale ModelInfo usage in multi_agent_mode.rs and
an outdated ModelClient::new call in prompt_debug_tests.rs. Complete log:
isolated dev tmp/harness-inference-pause-tests.log. Those files are untouched.
New tests now live in a standalone core/tests/inference_pause.rs target, using
the same helper-dispatch initializer. Fixed its ctor initializer to use a block;
that compile error is retained in the export's harness-test-target-compile.log.
Current standalone run log: isolated dev tmp/harness-inference-pause-standalone.log.
Fixtures use build_with_auto_env, target OS selection and executor filesystem
paths. Remote Wine/Docker execution itself requires Linux and is not observed here.
No competing Cargo build was
launched. There are eight new integration cases, including foreground PTY.
Full `just fmt` passed after
installing the repository's pinned DotSlash v0.5.8 (verified release SHA256) into
the isolated dev tools directory and using UV_NATIVE_TLS=true to use the Windows
certificate store. Initial full formatting failed on missing DotSlash and uv
certificate trust; both were resolved without disabling TLS verification.
Removed the earlier installed skill's poll-recipe routing: the requested fix
belongs in this harness. Restored only the unrelated full-formatter changes
(Bazel attribute order/CRLF-string escaping and normalized line endings), after
verifying those files were clean at task start and reviewing the exact changes.
Existing user prompt-profile edits are preserved.

Aggregate-output manual review (P0 under repository context rules): long waits
can combine formerly separate outputs into an item above 1K tokens. Added a hard
8K-token foreground budget, retaining existing truncation markers and leaving
space below the 10K item cap for headers. Prefixes are re-capped at each internal
cell observation; native output keeps the existing bounded head/tail buffer.
The noisy integration case requests an excessive budget and verifies the final
model-facing item stays capped. No instruction fragment is added to history.

Latest run: 5562b6aa-bc8c-4b69-9e6e-ca6055c4d8f8, 8 assertions passed, 2 native
cases flagged LEAK by nextest; do not call this a clean run. Checks include a
2.5-second native hold beyond the old Windows yield floor. Awaiting the command
UI end event before teardown removed the leak in isolated foreground run
53d494fe-bd15-4dd9-ab3d-317e59933c99, but parallel native warnings remain,
including the unchanged explicit-background path. No leak timeout was loosened.
Earlier assertion/mock-expectation failure and cleanup warning logs retained in
dev tmp/harness-inference-pause-before-fixture-fix.log and
dev tmp/harness-inference-pause-pass-with-leak.log. Runtime source behavior is
verified by all eight assertions; native test-process cleanup is a retained limit.

Schema generation completed successfully in cell 191 (39m04s cold/feature-variant
build); core/config.schema.json contains the new uint64 setting and description.
The CLI build completed successfully in cell 197/native handle 18692 (40m23s), launched with the
existing D:/dev/codex-dev/bin/build-codex-dev.ps1 wrapper, arguments `build
--profile dev-small --bin codex`. Complete log: dev tmp/harness-cli-build.log.
It rebuilds some dependencies because Cargo's dev/test/CLI feature unification
differs, despite the matched build profile. No competing Cargo build is active.
The loopback-only harness_smoke.py PASSED through that executable: native and
code-mode enabled cases each made two requests, with terminal output and exit 7
only on the resumed request (delays 6.875s and 6.375s). Disabled policy control
returned a pending handle (3.953s). Zero real model calls. Results in export
usage-patterns/harness-smoke-results.json; scratch evidence paths are recorded.
Copied the verified executable to the manifest's previously absent development
binary path, C:/Users/EthanO/.codex-dev/target/debug/codex.exe. SHA256:
493d2c8c605c832cd391f97abc93c0fe2a8530b51389cb0aa988486911d202a3.
Added tool_inference_pause_threshold_ms=10000 to the DEVELOPMENT home config,
preserving other fields. codex-dev.ps1 --version reports codex-cli 0.142.5.
Primary-home config, stock app binary and current desktop process are unchanged.

The repo-required `just fix -p codex-core --profile dev-small` completed with
exit 1 on the same five pre-existing aggregate-test compile errors. Full log:
dev tmp/harness-just-fix.log. Comparison against the pre-fix snapshot found only
one automatic change: removal of a redundant String::to_string copy in our
shell_spec.rs description. No behavior changed; the tested executable remains
equivalent. Existing unrelated Rust changes are byte-for-byte preserved.
Focused `cargo clippy --fix --allow-dirty --profile dev-small -p codex-core
--lib --test inference_pause` completed exit 0 in 1m43s. Seven warnings remain
in the pre-existing prompt_profiles.rs code; no feature lint warnings remain.
Log: dev tmp/harness-clippy-focused.log. Final snapshot comparison confirmed no
additional automatic edits. Pinned rustfmt check of the changed description and
git diff --check both passed. No tests were rerun after lint. Do not claim full
lint or full-suite success.
Full workspace tests need user approval under AGENTS.md and the unrelated
aggregate compilation errors resolved first. The strict ecosystem audit passed development
binary/home/shim checks; it reported unrelated missing skills-workshop, notify
chain and scheduled-task entries. Full JSON is dev tmp/harness-ecosystem-audit.json.
The schema helper now forwards optional Cargo flags so `just write-config-schema
--profile dev-small` can reuse the test build profile. Threshold windows are
capped at 60 seconds; 10 seconds is the intended development setting.
Two additional real-executable checks crossed that 60-second internal boundary:
native and code-mode commands each slept 65 seconds, then exited 7. Both PASSED;
the only resumed model request arrived with terminal output at 68.453s and
67.734s respectively, with no intermediate inference. Exact report in the export
usage-patterns/harness-long-wait-results.json. No real API/model calls were made.
No Rust test rerun is needed unless code changes or a remaining concern requires
it. Preserve current passing assertions and exact cleanup warnings. Primary desktop
uses stock 0.160.0; this canonical fork source identifies as 0.142.5, so activation
is limited to the isolated development harness. Do not overwrite the stock app.

DELIVERED: source implementation, isolated development activation and five
executable checks, including both 65-second boundary cases. Structured evidence:
artifacts/harness-waits/validation.json. No live build/test handles remain.
No commit, push, primary-config edit or stock desktop replacement was performed.
Remaining limits are the recorded Windows child-process cleanup warnings,
pre-existing aggregate compile blockers and unobserved remote/real-model runs;
no further implementation is pending for this development-harness slice.

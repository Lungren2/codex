# Foreground tool scheduling

The optional `tool_inference_pause_threshold_ms = 10000` setting keeps ordinary
commands and code cells pending inside the harness when they outlast the latency
threshold. Output events continue to the UI. The next model request receives
the terminal result; elapsed timers do not send it back to choose another poll.
This applies across command languages and to nested tools awaited by code mode.

The threshold is milliseconds, capped at 60000; unset or zero retains upstream yielding. It is
an initial scheduling window, not an execution timeout. Actual tool deadlines,
network denials, errors and turn cancellation retain their existing authority.
No in-flight model generation is forcibly cut off; the scheduler withholds the
next inference request while the foreground tool future remains unresolved.

Explicit `yield_time_ms: 0` selects background or interactive control. Foreground
PTY commands wait too; terminal allocation alone does not imply background work.
For a long-lived server launched inside code mode, use
zero yield at both the exec pragma and the nested shell call. The returned IDs
then retain existing wait, stdin and termination semantics. Ordinary finite
commands require no prompt recipe, repeated model polls, or language detection.

Enabled at 10000 milliseconds in the isolated development home. Launch
`codex-dev` to use the built fork (0.142.5). The current stock desktop harness
(0.160.0) is unchanged; it must run a binary containing the change to use it.

Validation: five executable cases passed (native, code mode, disabled control,
and two 65-second commands crossing the internal wait window). Eight integration
assertions passed, with two Windows child-process
cleanup warnings retained. Scoped Clippy passed; existing aggregate-test compile
errors block broader checks. See STATUS.md and validation.json for evidence.
No real-model token savings have been measured.

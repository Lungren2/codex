export type PhaseId =
  | "phase-1"
  | "phase-2a1"
  | "phase-2a2a"
  | "phase-2a2b"
  | "phase-2a3"
  | "phase-2b"
  | "phase-2c"
  | "phase-3"
  | "phase-4"
  | "phase-5"
  | "phase-6"
  | "phase-7"
  | "phase-8";

export type RouteLaneId = "architecture" | "implementation" | "journeyReview" | "codeQualityReview";

export type PhaseDefinition = {
  id: PhaseId;
  title: string;
  planFile: string;
  implementationLane: RouteLaneId;
  objective: string;
  requiredPaths: string[];
};

export const phases: PhaseDefinition[] = [
  {
    id: "phase-1",
    title: "Workflow-native worker invocation",
    planFile: "plan/phase-1-worker-api.html",
    implementationLane: "architecture",
    objective:
      "Land one durable workflow.run primitive for fresh and continuing workers without wrapping conversational subagent lifecycle tools.",
    requiredPaths: [
      "codex-rs/tools/src/tool_executor.rs",
      "codex-rs/core/src/tools/spec_plan.rs",
      "codex-rs/core/src/agent/control.rs",
      "codex-rs/core/src/tools/parallel.rs",
    ],
  },
  {
    id: "phase-2a1",
    title: "Trace stage model and replay",
    planFile: "plan/phase-2a1-stage-model.html",
    implementationLane: "architecture",
    objective:
      "Define bounded reservation and terminal stage events whose reducer replay enforces identity, parentage, deadlines, and monotonic attempts.",
    requiredPaths: [
      "codex-rs/rollout-trace/src/model/stage.rs",
      "codex-rs/rollout-trace/src/model/mod.rs",
      "codex-rs/rollout-trace/src/model/runtime.rs",
      "codex-rs/rollout-trace/src/raw_event.rs",
      "codex-rs/rollout-trace/src/reducer/mod.rs",
      "codex-rs/rollout-trace/src/reducer/code_cell.rs",
      "codex-rs/rollout-trace/src/reducer/stage.rs",
      "codex-rs/rollout-trace/src/reducer/stage_tests.rs",
      "codex-rs/rollout-trace/src/writer.rs",
      "codex-rs/rollout-trace/src/lib.rs",
    ],
  },
  {
    id: "phase-2a2a",
    title: "Durable writer prerequisite state",
    planFile: "plan/phase-2a2a-writer-state.html",
    implementationLane: "architecture",
    objective:
      "Track only synced lifecycle facts in a pure writer prerequisite index and preflight stage requests without exposing a writable stage producer.",
    requiredPaths: [
      "codex-rs/rollout-trace/src/stage.rs",
      "codex-rs/rollout-trace/src/writer.rs",
      "codex-rs/rollout-trace/src/writer/state.rs",
      "codex-rs/rollout-trace/src/writer_state_tests.rs",
    ],
  },
  {
    id: "phase-2a2b",
    title: "Required synced stage writer",
    planFile: "plan/phase-2a2b-synced-writer.html",
    implementationLane: "architecture",
    objective:
      "Expose the sole code-cell-bound stage producer with fail-closed payload and event syncing under the existing writer lock.",
    requiredPaths: [
      "codex-rs/rollout-trace/src/stage.rs",
      "codex-rs/rollout-trace/src/writer.rs",
      "codex-rs/rollout-trace/src/writer/stage.rs",
      "codex-rs/rollout-trace/src/writer/event_log.rs",
      "codex-rs/rollout-trace/src/code_cell.rs",
      "codex-rs/rollout-trace/src/writer_stage_tests.rs",
      "codex-rs/rollout-trace/src/lib.rs",
    ],
  },
  {
    id: "phase-2a3",
    title: "Default trace provisioning and exclusive recovery",
    planFile: "plan/phase-2a3-trace-recovery.html",
    implementationLane: "architecture",
    objective:
      "Provision authoritative traces from normal rollouts and allow only an exclusive root restart to recover proven abandoned stages.",
    requiredPaths: [
      "codex-rs/rollout-trace/src/recovery.rs",
      "codex-rs/rollout-trace/src/thread.rs",
      "codex-rs/rollout-trace/src/writer/reopen.rs",
      "codex-rs/rollout-trace/src/thread_tests.rs",
      "codex-rs/rollout-trace/src/recovery_tests.rs",
    ],
  },
  {
    id: "phase-2b",
    title: "Workflow lifecycle and parentage",
    planFile: "plan/phase-2b-workflow-lifecycle.html",
    implementationLane: "architecture",
    objective:
      "Make workflow stages terminalize before bounded cleanup, restore durable parentage, and prove live terminal and restart paths.",
    requiredPaths: [
      "codex-rs/core/src/workflow/invocation.rs",
      "codex-rs/core/src/agent/control.rs",
      "codex-rs/core/src/agent/control/spawn.rs",
      "codex-rs/core/tests/suite/workflow.rs",
    ],
  },
  {
    id: "phase-2c",
    title: "Code-mode runtime bounds",
    planFile: "plan/phase-2c-runtime-bounds.html",
    implementationLane: "architecture",
    objective:
      "Enforce every host-owned code-mode resource bound and prove each bound through table-driven and workflow integration coverage.",
    requiredPaths: [
      "codex-rs/code-mode-protocol/src/runtime.rs",
      "codex-rs/code-mode/src/session_runtime/mod.rs",
      "codex-rs/code-mode/src/session_runtime/tests.rs",
    ],
  },
  {
    id: "phase-3",
    title: "Restricted TypeScript authoring",
    planFile: "plan/phase-3-typescript-runtime.html",
    implementationLane: "architecture",
    objective:
      "Let an agent author a restricted TypeScript workflow that transpiles into the bounded code-mode runtime with auditable source artifacts.",
    requiredPaths: [
      "codex-rs/Cargo.toml",
      "codex-rs/code-mode/Cargo.toml",
      "codex-rs/code-mode/src/runtime/module_loader.rs",
      "codex-rs/code-mode/src/runtime/mod.rs",
      "codex-rs/rollout-trace/src/code_cell.rs",
      "MODULE.bazel.lock",
    ],
  },
  {
    id: "phase-4",
    title: "Bounded context projections",
    planFile: "plan/phase-4-context-projections.html",
    implementationLane: "architecture",
    objective:
      "Add bounded, provenance-carrying context projections without transcript dumping or routing policy.",
    requiredPaths: [
      "codex-rs/protocol/src/protocol.rs",
      "codex-rs/core/src/context/contextual_user_message.rs",
      "codex-rs/core/src/agent/control/spawn.rs",
      "codex-rs/app-server-protocol/src/protocol/v2/turn.rs",
    ],
  },
  {
    id: "phase-5",
    title: "Capability routing",
    planFile: "plan/phase-5-capability-routing.html",
    implementationLane: "architecture",
    objective:
      "Compile host-owned capability lanes into immutable route decisions with alternatives and evidence-driven escalation.",
    requiredPaths: [
      "codex-rs/core/src/session/turn_context.rs",
      "codex-rs/app-server-protocol/src/protocol/v2/turn.rs",
      "sdk/typescript/src/threadOptions.ts",
      "sdk/typescript/samples/workflow_orchestrator/runtime.ts",
    ],
  },
  {
    id: "phase-6",
    title: "Intent resolution",
    planFile: "plan/phase-6-intent-resolution.html",
    implementationLane: "implementation",
    objective:
      "Add a bounded intent contract that challenges assumptions and interrupts the user only for material ambiguity.",
    requiredPaths: [
      "codex-rs/core/src/tools/handlers/request_user_input.rs",
      "codex-rs/protocol/src/request_user_input.rs",
      "codex-rs/core/src/context/contextual_user_message.rs",
      "sdk/typescript/samples/workflow_orchestrator/contracts.ts",
    ],
  },
  {
    id: "phase-7",
    title: "Mission-first journey verification",
    planFile: "plan/phase-7-mission-verification.html",
    implementationLane: "implementation",
    objective:
      "Separate inline progress probes from independent journey judgment and require functional evidence before code-quality review.",
    requiredPaths: [
      "codex-rs/app-server-protocol/src/protocol/v2/item.rs",
      "codex-rs/app-server/src/request_processors/turn_processor.rs",
      "sdk/typescript/src/events.ts",
      "sdk/typescript/samples/workflow_orchestrator/runtime.ts",
    ],
  },
  {
    id: "phase-8",
    title: "Code quality and landing",
    planFile: "plan/phase-8-quality-landing.html",
    implementationLane: "architecture",
    objective:
      "Add independent structured code-quality judgment and an explicitly requested deterministic landing gate after the user journey works.",
    requiredPaths: [
      "codex-rs/app-server-protocol/src/protocol/v2/review.rs",
      "codex-rs/core/src/session/review.rs",
      "codex-rs/app-server/src/request_processors/turn_processor.rs",
      "sdk/typescript/samples/workflow_orchestrator/runtime.ts",
    ],
  },
];

export function selectPhases(from?: PhaseId, through?: PhaseId): PhaseDefinition[] {
  const startIndex = from ? phases.findIndex((phase) => phase.id === from) : 0;
  const endIndex = through ? phases.findIndex((phase) => phase.id === through) : phases.length - 1;
  if (startIndex < 0) throw new Error(`unknown --from phase: ${from}`);
  if (endIndex < 0) throw new Error(`unknown --through phase: ${through}`);
  if (endIndex < startIndex) {
    throw new Error(`--through phase ${through} precedes --from phase ${from}`);
  }
  return phases.slice(startIndex, endIndex + 1);
}

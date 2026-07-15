export type PhaseId =
  | "phase-1"
  | "phase-2"
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
    id: "phase-2",
    title: "Stage kernel and result envelope",
    planFile: "plan/phase-2-stage-kernel.html",
    implementationLane: "architecture",
    objective:
      "Make every workflow invocation produce a bounded, durable StageResult with authoritative execution metadata and cancellation.",
    requiredPaths: [
      "codex-rs/code-mode-protocol/src/runtime.rs",
      "codex-rs/code-mode/src/session_runtime/mod.rs",
      "codex-rs/rollout-trace/src/model/runtime.rs",
      "codex-rs/rollout-trace/src/code_cell.rs",
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

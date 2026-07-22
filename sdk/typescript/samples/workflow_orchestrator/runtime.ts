import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";

import { Codex } from "@openai/codex-sdk";
import type { ModelReasoningEffort, Thread, Usage } from "@openai/codex-sdk";
import type { ZodType } from "zod";

import { implementationOutputSchema, jsonSchemaFor, reviewOutputSchema } from "./contracts.ts";
import type { ImplementationOutput, ReviewOutput } from "./contracts.ts";
import { anchorGitTree, captureGitTree, diffGitTrees, sameGitState, snapshotGit } from "./git.ts";
import type { GitSnapshot } from "./git.ts";
import type { PhaseDefinition, RouteLaneId } from "./phases.ts";

export type CapabilityProfile = {
  id: string;
  model: string;
  reasoningEffort: ModelReasoningEffort;
  costRank: number;
  capabilities: Record<string, number>;
};

export type RoutingLane = {
  minimum: Record<string, number>;
  affinity: string[][];
  escalation: string[];
};

export type RoutingGuide = {
  source: string;
  profiles: CapabilityProfile[];
  lanes: Record<RouteLaneId, RoutingLane>;
};

export type RouteDecision = {
  lane: RouteLaneId;
  profileId: string;
  model: string;
  reasoningEffort: ModelReasoningEffort;
  reason: string;
  alternatives: Array<{
    profileId: string;
    model: string;
    reasoningEffort: ModelReasoningEffort;
    rejectedBecause: string;
  }>;
  escalationAllowed: boolean;
};

type SessionRole = "implementation" | "repair" | "journey_review" | "code_quality_review";
type SessionStatus =
  | "running"
  | "completed"
  | "failed"
  | "timed_out"
  | "policy_violation"
  | "interrupted";
type ValidationMode = "journey_review" | "code_quality_review";
type GranularityMode = "fresh_thread" | "continue_implementation";

type ReviewInvocation = {
  validationMode: ValidationMode;
  granularity: GranularityMode;
  routeLane: RouteLaneId;
};

type SessionResult = {
  id: string;
  phaseId: string;
  role: SessionRole;
  status: SessionStatus;
  threadId: string | null;
  route: RouteDecision;
  output: ImplementationOutput | ReviewOutput | null;
  error: string | null;
  usage: Usage | null;
  gitBefore: GitSnapshot;
  gitAfter: GitSnapshot | null;
  startedAt: string;
  finishedAt: string | null;
};

type ImplementationRun = {
  attemptIds: string[];
  result: SessionResult & { output: ImplementationOutput };
  thread: Thread;
  route: RouteDecision;
  recoveries: number;
};

type PhaseResult = {
  phaseId: string;
  accepted: boolean;
  sessionIds: string[];
  handoff: string;
  projection: {
    decisions: ImplementationOutput["decisions"];
    verification: string[];
    changedFiles: string[];
    finalPatchHash: string;
  } | null;
};

type WorkflowLimits = {
  maxSessions: number;
  maxRepairsPerPhase: number;
  maxSessionMs: number;
  maxRunMs: number;
};

type PhaseBaseline = {
  treeHash: string;
  refName: string;
  git: GitSnapshot;
  capturedAt: string;
};

type PlanDocument = {
  planFile: string;
  sha256: string;
  content: string;
  definitionSha256: string;
  definition: PhaseDefinition;
};

type WorkflowState = {
  runId: string;
  repoRoot: string;
  startedAt: string;
  deadlineAt: string;
  limits: WorkflowLimits;
  routingPolicy: {
    hash: string;
    guide: RoutingGuide;
  };
  sessionsUsed: number;
  attempts: SessionResult[];
  phaseOrder: string[];
  planDocuments: Record<string, PlanDocument>;
  phaseBaselines: Record<string, PhaseBaseline>;
  phases: PhaseResult[];
};

export type RuntimeOptions = WorkflowLimits & {
  repoRoot: string;
  planRoot: string;
  stateFile: string;
  routing: RoutingGuide;
};

const MAX_PLAN_CHARS = 12_000;
const MAX_HANDOFF_CHARS = 12_000;
const MAX_HANDOFF_PROJECTION_CHARS = 8_000;
const MAX_SESSION_PROMPT_CHARS = 36_000;

const JOURNEY_REVIEW: ReviewInvocation = {
  validationMode: "journey_review",
  granularity: "fresh_thread",
  routeLane: "journeyReview",
};
const CODE_QUALITY_REVIEW: ReviewInvocation = {
  validationMode: "code_quality_review",
  granularity: "fresh_thread",
  routeLane: "codeQualityReview",
};

export class WorkflowRuntime {
  private readonly codex: Codex;
  private readonly options: RuntimeOptions;
  private readonly state: WorkflowState;
  private readonly deadline: number;

  constructor(options: RuntimeOptions, runId: string) {
    this.options = options;
    this.codex = new Codex({
      codexPathOverride: process.env.CODEX_EXECUTABLE,
      env: definedEnvironment(),
    });

    const loaded = loadState(options.stateFile);
    const startedAt = loaded?.startedAt ?? new Date().toISOString();
    const limits = loaded?.limits ?? limitsFromOptions(options);
    const routingPolicy = {
      hash: routingHash(options.routing),
      guide: options.routing,
    };
    this.state = loaded
      ? {
          runId: loaded.runId,
          repoRoot: loaded.repoRoot,
          startedAt,
          deadlineAt:
            loaded.deadlineAt ?? new Date(Date.parse(startedAt) + limits.maxRunMs).toISOString(),
          limits,
          routingPolicy: loaded.routingPolicy ?? routingPolicy,
          sessionsUsed: loaded.sessionsUsed ?? 0,
          attempts: loaded.attempts ?? [],
          phaseOrder: loaded.phaseOrder ?? [],
          planDocuments: loaded.planDocuments ?? {},
          phaseBaselines: loaded.phaseBaselines ?? {},
          phases: loaded.phases ?? [],
        }
      : {
          runId,
          repoRoot: options.repoRoot,
          startedAt,
          deadlineAt: new Date(Date.parse(startedAt) + limits.maxRunMs).toISOString(),
          limits,
          routingPolicy,
          sessionsUsed: 0,
          attempts: [],
          phaseOrder: [],
          planDocuments: {},
          phaseBaselines: {},
          phases: [],
        };

    if (this.state.runId !== runId) {
      throw new Error(`ledger runId ${this.state.runId} does not match requested runId ${runId}`);
    }
    if (path.resolve(this.state.repoRoot) !== path.resolve(options.repoRoot)) {
      throw new Error("a workflow run cannot be resumed against a different repository");
    }
    if (this.state.routingPolicy.hash !== routingPolicy.hash) {
      throw new Error(
        "routing policy changed after the run started; use a new runId for an amended policy",
      );
    }
    if (routingHash(this.state.routingPolicy.guide) !== this.state.routingPolicy.hash) {
      throw new Error("persisted routing policy does not match its recorded hash");
    }

    this.deadline = Date.parse(this.state.deadlineAt);
    this.recoverInterruptedAttempts();
    this.persist();
  }

  async run(phases: PhaseDefinition[]): Promise<void> {
    const pinnedPhases = this.pinPlanDocuments(phases);
    for (const phase of pinnedPhases) {
      if (this.state.phases.some((result) => result.phaseId === phase.id && result.accepted)) {
        continue;
      }
      const result = await this.runPhase(phase);
      this.state.phases.push(result);
      this.persist();
      if (!result.accepted) {
        throw new Error(`${phase.id} did not clear its phase gate; see ${this.options.stateFile}`);
      }
    }
  }

  private async runPhase(phase: PhaseDefinition): Promise<PhaseResult> {
    const sessionIds: string[] = [];
    const phaseBaseline = this.phaseBaseline(phase.id);
    const plan = this.state.planDocuments[phase.id]?.content;
    if (!plan) throw new Error(`phase document ${phase.id} was not pinned before execution`);
    const priorAttemptEvidence = this.phaseAttemptProjection(phase.id);

    const priorWritableAttempts = writableAttemptsForPhase(this.state.attempts, phase.id);
    let repairs = repairBudgetUsedForPhase(this.state.attempts, phase.id);
    if (repairs >= this.state.limits.maxRepairsPerPhase && this.hasUnresolvedReview(phase.id)) {
      return phaseFailure(
        phase.id,
        sessionIds,
        `phase repair budget exhausted (${repairs}/${this.state.limits.maxRepairsPerPhase})`,
      );
    }

    const reusableImplementation = this.reusableImplementationAttempt(phase.id);
    let currentRoute: RouteDecision;
    let currentThread: Thread;
    let currentImplementation: SessionResult & { output: ImplementationOutput };
    let implementationRun: ImplementationRun;
    if (reusableImplementation) {
      currentRoute = reusableImplementation.route;
      currentThread = this.resumeThread(
        reusableImplementation.threadId,
        currentRoute,
        "workspace-write",
      );
      currentImplementation = reusableImplementation;
      sessionIds.push(reusableImplementation.id);
    } else {
      if (priorWritableAttempts > 0) {
        if (repairs >= this.state.limits.maxRepairsPerPhase) {
          return phaseFailure(
            phase.id,
            sessionIds,
            `phase repair budget exhausted (${repairs}/${this.state.limits.maxRepairsPerPhase})`,
          );
        }
        repairs += 1;
      }

      currentRoute = selectRoute(this.state.routingPolicy.guide, phase.implementationLane);
      currentThread = this.newThread(currentRoute, "workspace-write");
      implementationRun = await this.runImplementationWithRecovery(
        phase,
        plan,
        "implementation",
        priorAttemptEvidence,
        currentThread,
        currentRoute,
        this.state.limits.maxRepairsPerPhase - repairs,
      );
      currentImplementation = implementationRun.result;
      currentThread = implementationRun.thread;
      currentRoute = implementationRun.route;
      sessionIds.push(...implementationRun.attemptIds);
      if (currentImplementation.output.status !== "completed") {
        return phaseFailure(phase.id, sessionIds, currentImplementation.output.handoff);
      }

      repairs += implementationRun.recoveries;
    }
    while (true) {
      const journey = await this.runReview(
        phase,
        plan,
        currentImplementation.output,
        phaseBaseline,
        JOURNEY_REVIEW,
        currentThread,
      );
      sessionIds.push(journey.id);

      if (journey.output.decision !== "accepted") {
        if (
          journey.output.decision === "blocked" ||
          !isImplementationRepair(journey.output) ||
          repairs >= this.state.limits.maxRepairsPerPhase
        ) {
          return phaseFailure(phase.id, sessionIds, journey.output.summary);
        }
        repairs += 1;
        ({ thread: currentThread, route: currentRoute } = this.repairInvocation(
          phase.implementationLane,
          currentThread,
          currentRoute,
          journey.output,
        ));
        implementationRun = await this.runImplementationWithRecovery(
          phase,
          plan,
          "repair",
          JSON.stringify(reviewProjection(journey.output)),
          currentThread,
          currentRoute,
          this.state.limits.maxRepairsPerPhase - repairs,
        );
        currentImplementation = implementationRun.result;
        currentThread = implementationRun.thread;
        currentRoute = implementationRun.route;
        repairs += implementationRun.recoveries;
        sessionIds.push(...implementationRun.attemptIds);
        if (currentImplementation.output.status !== "completed") {
          return phaseFailure(phase.id, sessionIds, currentImplementation.output.handoff);
        }
        continue;
      }

      const quality = await this.runReview(
        phase,
        plan,
        currentImplementation.output,
        phaseBaseline,
        CODE_QUALITY_REVIEW,
        currentThread,
      );
      sessionIds.push(quality.id);
      if (quality.output.decision === "accepted") {
        const finalTree = captureGitTree(this.options.repoRoot);
        const finalDelta = diffGitTrees(this.options.repoRoot, phaseBaseline.treeHash, finalTree);
        return {
          phaseId: phase.id,
          accepted: true,
          sessionIds,
          handoff: currentImplementation.output.handoff.slice(0, MAX_HANDOFF_CHARS),
          projection: {
            decisions: currentImplementation.output.decisions,
            verification: currentImplementation.output.verification,
            changedFiles: finalDelta.changedFiles,
            finalPatchHash: finalDelta.patchHash,
          },
        };
      }
      if (
        quality.output.decision === "blocked" ||
        !isImplementationRepair(quality.output) ||
        repairs >= this.state.limits.maxRepairsPerPhase
      ) {
        return phaseFailure(phase.id, sessionIds, quality.output.summary);
      }

      repairs += 1;
      ({ thread: currentThread, route: currentRoute } = this.repairInvocation(
        phase.implementationLane,
        currentThread,
        currentRoute,
        quality.output,
      ));
      implementationRun = await this.runImplementationWithRecovery(
        phase,
        plan,
        "repair",
        JSON.stringify(reviewProjection(quality.output)),
        currentThread,
        currentRoute,
        this.state.limits.maxRepairsPerPhase - repairs,
      );
      currentImplementation = implementationRun.result;
      currentThread = implementationRun.thread;
      currentRoute = implementationRun.route;
      repairs += implementationRun.recoveries;
      sessionIds.push(...implementationRun.attemptIds);
      if (currentImplementation.output.status !== "completed") {
        return phaseFailure(phase.id, sessionIds, currentImplementation.output.handoff);
      }
      // The loop deliberately returns to journey review after every behavioral or quality repair.
    }
  }

  private async runImplementation(
    thread: Thread,
    route: RouteDecision,
    phase: PhaseDefinition,
    plan: string,
    role: "implementation" | "repair",
    findings: string,
  ): Promise<SessionResult & { output: ImplementationOutput }> {
    const prompt = `${plan}\n\nPHASE OBJECTIVE\n${phase.objective}\n\nREQUIRED SURFACES\n${phase.requiredPaths.join("\n")}\n\nPRIOR ACCEPTED HANDOFFS\n${this.handoffProjection()}\n\nHOST ROUTE DECISION\n${JSON.stringify(route)}\n\nPRIOR PHASE ATTEMPT EVIDENCE\n${findings || "None."}\n\nImplement only this phase. Work depth-first, follow every applicable AGENTS.md, and do not spawn subagents. Preserve user changes. The mission is the working feature slice; use progress probes while building, then run repository-required formatting and focused tests after it lands. Return the contract only.`;
    return this.runSession(phase.id, thread, route, role, prompt, implementationOutputSchema);
  }

  private async runImplementationWithRecovery(
    phase: PhaseDefinition,
    plan: string,
    role: "implementation" | "repair",
    findings: string,
    initialThread: Thread,
    initialRoute: RouteDecision,
    maxRecoveries: number,
  ): Promise<ImplementationRun> {
    let thread = initialThread;
    let route = initialRoute;
    let recoveryFindings = findings;
    const attemptIds: string[] = [];

    for (let recoveries = 0; ; recoveries += 1) {
      try {
        const result = await this.runImplementation(
          thread,
          route,
          phase,
          plan,
          role,
          recoveryFindings,
        );
        attemptIds.push(result.id);
        return { attemptIds, result, thread, route, recoveries };
      } catch (error) {
        const failedAttempt = this.state.attempts.at(-1);
        if (
          !failedAttempt ||
          failedAttempt.phaseId !== phase.id ||
          failedAttempt.role !== role ||
          failedAttempt.status === "running"
        ) {
          throw error;
        }
        attemptIds.push(failedAttempt.id);
        if (recoveries >= maxRecoveries) {
          const output = exhaustedRecoveryOutput({
            phaseId: phase.id,
            attemptId: failedAttempt.id,
            status: failedAttempt.status,
            error: failedAttempt.error ?? errorMessage(error),
          });
          return {
            attemptIds,
            result: { ...failedAttempt, output },
            thread,
            route,
            recoveries,
          };
        }

        recoveryFindings = JSON.stringify({
          priorFindings: recoveryFindings || null,
          hostRecovery: {
            failedAttempt: failedAttempt.id,
            status: failedAttempt.status,
            error: failedAttempt.error,
            changedFiles: failedAttempt.gitAfter?.changedFiles ?? [],
            diffStat: failedAttempt.gitAfter?.diffStat ?? "",
            patchHash: failedAttempt.gitAfter?.patchHash ?? null,
            instruction:
              "Continue from the existing working tree. Inspect and preserve valid work, finish the phase, run only the required focused checks, and return the contract.",
          },
        });
        thread = this.newThread(route, "workspace-write");
      }
    }
  }

  private async runReview(
    phase: PhaseDefinition,
    plan: string,
    implementation: ImplementationOutput,
    phaseBaseline: PhaseBaseline,
    invocation: ReviewInvocation,
    implementationThread: Thread,
  ): Promise<SessionResult & { output: ReviewOutput }> {
    const route = selectRoute(this.state.routingPolicy.guide, invocation.routeLane);
    const thread =
      invocation.granularity === "fresh_thread"
        ? this.newThread(route, "read-only")
        : implementationThread;
    const currentTree = captureGitTree(this.options.repoRoot);
    const phaseDelta = diffGitTrees(this.options.repoRoot, phaseBaseline.treeHash, currentTree);
    const focus =
      invocation.validationMode === "journey_review"
        ? "Trace the implemented path and decide whether the phase's usable capability actually works. Require runtime evidence when available, otherwise a bounded entry-to-effect trace. Treat tests as evidence, not the definition of success."
        : "Review correctness, integration, API shape, maintainability, and unnecessary complexity only after accepting that the functional slice exists.";
    const prompt = `${plan}\n\nPHASE\n${phase.title}: ${phase.objective}\n\nVALIDATION MODE\n${invocation.validationMode}\n\nGRANULARITY\n${invocation.granularity}\n\nIMPLEMENTER CLAIM\n${JSON.stringify(implementationProjection(implementation))}\n\nAUTHORITATIVE PHASE DELTA\n${JSON.stringify(phaseDelta)}\n\n${focus}\nClassify a rejection as capability, implementation_defect, environment, or scope; accepted work uses failureClass none. Do not modify files and do not spawn subagents. Return the contract only.`;
    return this.runSession(
      phase.id,
      thread,
      route,
      invocation.validationMode,
      prompt,
      reviewOutputSchema,
      true,
    );
  }

  private async runSession<T extends ImplementationOutput | ReviewOutput>(
    phaseId: string,
    thread: Thread,
    route: RouteDecision,
    role: SessionRole,
    prompt: string,
    schema: ZodType<T>,
    enforceReadOnly = false,
  ): Promise<SessionResult & { output: T }> {
    if (prompt.length > MAX_SESSION_PROMPT_CHARS) {
      throw new Error(`${role} prompt exceeds the ${MAX_SESSION_PROMPT_CHARS}-character bound`);
    }
    this.assertWithinBounds();
    const gitBefore = snapshotGit(this.options.repoRoot);
    const attempt: SessionResult = {
      id: `${phaseId}:${role}:${this.state.sessionsUsed + 1}`,
      phaseId,
      role,
      status: "running",
      threadId: thread.id,
      route,
      output: null,
      error: null,
      usage: null,
      gitBefore,
      gitAfter: null,
      startedAt: new Date().toISOString(),
      finishedAt: null,
    };
    this.state.sessionsUsed += 1;
    this.state.attempts.push(attempt);
    this.persist();

    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), this.state.limits.maxSessionMs);
    try {
      const turn = await thread.run(prompt, {
        outputSchema: jsonSchemaFor(schema),
        signal: controller.signal,
      });
      const output = schema.parse(JSON.parse(turn.finalResponse));
      const gitAfter = snapshotGit(this.options.repoRoot);
      if (enforceReadOnly && !sameGitState(gitBefore, gitAfter)) {
        throw new Error(`${role} modified the repository during a read-only session`);
      }
      attempt.status = "completed";
      attempt.threadId = thread.id;
      attempt.output = output;
      attempt.usage = turn.usage;
      attempt.gitAfter = gitAfter;
      attempt.finishedAt = new Date().toISOString();
      this.persist();
      return { ...attempt, output };
    } catch (error) {
      attempt.status = controller.signal.aborted
        ? "timed_out"
        : errorMessage(error).includes("read-only session")
          ? "policy_violation"
          : "failed";
      attempt.error = errorMessage(error);
      attempt.threadId = thread.id;
      attempt.gitAfter = trySnapshotGit(this.options.repoRoot);
      attempt.finishedAt = new Date().toISOString();
      this.persist();
      throw error;
    } finally {
      clearTimeout(timeout);
    }
  }

  private repairInvocation(
    lane: RouteLaneId,
    thread: Thread,
    route: RouteDecision,
    review: ReviewOutput,
  ): { thread: Thread; route: RouteDecision } {
    if (review.failureClass !== "capability") return { thread, route };
    const escalated = selectEscalation(this.state.routingPolicy.guide, lane, route.profileId);
    return escalated.profileId === route.profileId
      ? { thread, route }
      : { thread: this.newThread(escalated, "workspace-write"), route: escalated };
  }

  private newThread(route: RouteDecision, sandboxMode: "read-only" | "workspace-write"): Thread {
    return this.codex.startThread({
      model: route.model,
      modelReasoningEffort: route.reasoningEffort,
      sandboxMode,
      approvalPolicy: "never",
      workingDirectory: this.options.repoRoot,
      skipGitRepoCheck: false,
    });
  }

  private resumeThread(
    threadId: string,
    route: RouteDecision,
    sandboxMode: "read-only" | "workspace-write",
  ): Thread {
    return this.codex.resumeThread(threadId, {
      model: route.model,
      modelReasoningEffort: route.reasoningEffort,
      sandboxMode,
      approvalPolicy: "never",
      workingDirectory: this.options.repoRoot,
      skipGitRepoCheck: false,
    });
  }

  private reusableImplementationAttempt(
    phaseId: string,
  ): (SessionResult & { output: ImplementationOutput; threadId: string }) | null {
    const attempt = [...this.state.attempts]
      .reverse()
      .find(
        (candidate) =>
          candidate.phaseId === phaseId &&
          (candidate.role === "implementation" || candidate.role === "repair") &&
          candidate.status === "completed",
      );
    if (
      !attempt?.output ||
      "decision" in attempt.output ||
      !attempt.threadId ||
      !attempt.gitAfter ||
      attempt.gitAfter.patchHash !== snapshotGit(this.options.repoRoot).patchHash
    ) {
      return null;
    }
    return { ...attempt, output: attempt.output, threadId: attempt.threadId };
  }

  private hasUnresolvedReview(phaseId: string): boolean {
    const attempt = [...this.state.attempts]
      .reverse()
      .find((candidate) => candidate.phaseId === phaseId && candidate.status === "completed");
    return Boolean(
      attempt?.output && "decision" in attempt.output && attempt.output.decision !== "accepted",
    );
  }

  private phaseBaseline(phaseId: string): PhaseBaseline {
    const existing = this.state.phaseBaselines[phaseId];
    if (existing) return existing;
    const treeHash = captureGitTree(this.options.repoRoot);
    const refName = `refs/codex/workflow-baselines/${this.state.runId}/${phaseId}`;
    anchorGitTree(this.options.repoRoot, refName, treeHash);
    const baseline = {
      treeHash,
      refName,
      git: snapshotGit(this.options.repoRoot),
      capturedAt: new Date().toISOString(),
    };
    this.state.phaseBaselines[phaseId] = baseline;
    this.persist();
    return baseline;
  }

  private pinPlanDocuments(phases: PhaseDefinition[]): PhaseDefinition[] {
    let changed = false;
    const requestedPhaseIds = phases.map((phase) => phase.id);
    if (this.state.phaseOrder.length === 0) {
      this.state.phaseOrder = requestedPhaseIds;
      changed = true;
    } else {
      selectPinnedPhaseIds(this.state.phaseOrder, requestedPhaseIds);
    }
    for (const phase of phases) {
      const existing = this.state.planDocuments[phase.id];
      if (existing) {
        if (createHash("sha256").update(existing.content).digest("hex") !== existing.sha256) {
          throw new Error(`persisted phase document ${phase.id} does not match its recorded hash`);
        }
        if (phaseDefinitionHash(existing.definition) !== existing.definitionSha256) {
          throw new Error(
            `persisted phase definition ${phase.id} does not match its recorded hash`,
          );
        }
        continue;
      }
      const content = fs.readFileSync(path.join(this.options.planRoot, phase.planFile), "utf8");
      if (content.length > MAX_PLAN_CHARS) {
        throw new Error(`${phase.planFile} exceeds the ${MAX_PLAN_CHARS}-character context bound`);
      }
      this.state.planDocuments[phase.id] = {
        planFile: phase.planFile,
        sha256: createHash("sha256").update(content).digest("hex"),
        content,
        definitionSha256: phaseDefinitionHash(phase),
        definition: phase,
      };
      changed = true;
    }
    if (changed) this.persist();
    return requestedPhaseIds.map((phaseId) => {
      const pinned = this.state.planDocuments[phaseId];
      if (!pinned) throw new Error(`missing pinned definition for phase ${phaseId}`);
      return pinned.definition;
    });
  }

  private handoffProjection(): string {
    const projection = this.state.phases
      .filter((phase) => phase.accepted)
      .map(({ phaseId, handoff, projection: evidence }) => ({
        phaseId,
        handoff: handoff.slice(0, 2_000),
        evidence: evidence
          ? {
              decisions: evidence.decisions.slice(0, 4),
              verification: evidence.verification.slice(0, 4),
              changedFiles: evidence.changedFiles.slice(0, 100),
              finalPatchHash: evidence.finalPatchHash,
            }
          : null,
      }));
    while (
      projection.length > 1 &&
      JSON.stringify(projection).length > MAX_HANDOFF_PROJECTION_CHARS
    ) {
      projection.shift();
    }
    const serialized = JSON.stringify(projection);
    if (serialized.length <= MAX_HANDOFF_PROJECTION_CHARS) return serialized;
    const latest = projection.at(-1);
    return JSON.stringify(
      latest ? [{ phaseId: latest.phaseId, handoff: latest.handoff, evidence: null }] : [],
    );
  }

  private phaseAttemptProjection(phaseId: string): string {
    const projection = this.state.attempts
      .filter((attempt) => attempt.phaseId === phaseId && attempt.status !== "running")
      .slice(-4)
      .map((attempt) => ({
        id: attempt.id,
        role: attempt.role,
        status: attempt.status,
        error: attempt.error,
        output: attempt.output
          ? "decision" in attempt.output
            ? reviewProjection(attempt.output)
            : implementationProjection(attempt.output)
          : null,
        gitAfter: attempt.gitAfter
          ? {
              changedFiles: attempt.gitAfter.changedFiles.slice(0, 40),
              diffStat: attempt.gitAfter.diffStat.slice(0, 2_000),
              patchHash: attempt.gitAfter.patchHash,
            }
          : null,
      }));
    while (
      projection.length > 1 &&
      JSON.stringify(projection).length > MAX_HANDOFF_PROJECTION_CHARS
    ) {
      projection.shift();
    }
    const serialized = JSON.stringify(projection);
    return serialized.length <= MAX_HANDOFF_PROJECTION_CHARS
      ? serialized
      : serialized.slice(0, MAX_HANDOFF_PROJECTION_CHARS);
  }

  private assertWithinBounds(): void {
    if (this.state.sessionsUsed >= this.state.limits.maxSessions) {
      throw new Error(`workflow exceeded ${this.state.limits.maxSessions} sessions`);
    }
    if (Date.now() >= this.deadline) {
      throw new Error(`workflow exceeded its persisted deadline ${this.state.deadlineAt}`);
    }
  }

  private recoverInterruptedAttempts(): void {
    const interruptedAt = new Date().toISOString();
    for (const attempt of this.state.attempts) {
      if (attempt.status !== "running") continue;
      attempt.status = "interrupted";
      attempt.error = "runner exited before the session produced a terminal result";
      attempt.finishedAt = interruptedAt;
      attempt.gitAfter = trySnapshotGit(this.options.repoRoot);
    }
    this.state.sessionsUsed = Math.max(this.state.sessionsUsed, this.state.attempts.length);
  }

  private persist(): void {
    fs.mkdirSync(path.dirname(this.options.stateFile), { recursive: true });
    const temporary = `${this.options.stateFile}.${process.pid}.tmp`;
    fs.writeFileSync(temporary, `${JSON.stringify(this.state, null, 2)}\n`, "utf8");
    fs.renameSync(temporary, this.options.stateFile);
  }
}

export function maxSessionsForWorkflow(phaseCount: number, maxRepairsPerPhase: number): number {
  return phaseCount * (3 + 3 * maxRepairsPerPhase);
}

export function selectPinnedPhaseIds(
  pinnedPhaseOrder: string[],
  requestedPhaseIds: string[],
): string[] {
  const firstPhaseId = requestedPhaseIds[0];
  if (!firstPhaseId) throw new Error("a workflow run requires at least one phase");
  if (pinnedPhaseOrder.length === 0) return requestedPhaseIds;
  const startIndex = pinnedPhaseOrder.indexOf(firstPhaseId);
  const pinnedSelection = pinnedPhaseOrder.slice(startIndex, startIndex + requestedPhaseIds.length);
  if (startIndex < 0 || !sameStringArray(pinnedSelection, requestedPhaseIds)) {
    throw new Error("requested phases are not a contiguous selection of the pinned phase order");
  }
  return requestedPhaseIds;
}

export function repairBudgetUsedForPhase(
  attempts: Array<Pick<SessionResult, "phaseId" | "role">>,
  phaseId: string,
): number {
  return Math.max(0, writableAttemptsForPhase(attempts, phaseId) - 1);
}

export function exhaustedRecoveryOutput(args: {
  phaseId: string;
  attemptId: string;
  status: string;
  error: string;
}): ImplementationOutput {
  const summary = `${args.phaseId} exhausted its bounded implementation recovery budget.`;
  const detail = `${args.attemptId} ended as ${args.status}: ${args.error}`;
  return {
    status: "blocked",
    summary,
    decisions: [],
    verification: [],
    blockingIssue: detail,
    handoff: `${summary} ${detail}`,
  };
}

export function assertRoutingGuide(value: RoutingGuide): void {
  const efforts = new Set(["minimal", "low", "medium", "high", "xhigh"]);
  if (!value.source || !Array.isArray(value.profiles) || value.profiles.length === 0) {
    throw new Error("routing guide must identify its source and provide profiles");
  }
  const profileIds = new Set<string>();
  for (const profile of value.profiles) {
    if (
      !profile.id ||
      profileIds.has(profile.id) ||
      !profile.model ||
      !efforts.has(profile.reasoningEffort) ||
      !Number.isFinite(profile.costRank)
    ) {
      throw new Error(`invalid routing profile: ${profile.id || "<missing id>"}`);
    }
    profileIds.add(profile.id);
  }
  for (const laneId of routeLaneIds()) {
    const lane = value.lanes?.[laneId];
    if (!lane || lane.affinity.length === 0) {
      throw new Error(`routing lane ${laneId} must provide capability minimums and affinity`);
    }
    for (const profileId of [...lane.affinity.flat(), ...lane.escalation]) {
      if (!profileIds.has(profileId)) {
        throw new Error(`routing lane ${laneId} references unknown profile ${profileId}`);
      }
    }
    selectRoute(value, laneId);
  }
}

export function selectRoute(guide: RoutingGuide, laneId: RouteLaneId): RouteDecision {
  const lane = guide.lanes[laneId];
  const eligible = guide.profiles.filter((profile) => meetsMinimum(profile, lane.minimum));
  if (eligible.length === 0) throw new Error(`no profile satisfies routing lane ${laneId}`);

  const ranked = [...eligible].sort((left, right) => {
    const affinity = affinityRank(lane, left.id) - affinityRank(lane, right.id);
    return affinity !== 0 ? affinity : left.costRank - right.costRank;
  });
  const selected = ranked[0];
  if (!selected) throw new Error(`no profile satisfies routing lane ${laneId}`);
  return routeDecision(
    laneId,
    selected,
    ranked.slice(1),
    `Selected ${selected.id}: capability minimums passed; lane affinity preceded cost rank.`,
    lane.escalation.length > 0,
  );
}

function selectEscalation(
  guide: RoutingGuide,
  laneId: RouteLaneId,
  currentProfileId: string,
): RouteDecision {
  const lane = guide.lanes[laneId];
  const profiles = new Map(guide.profiles.map((profile) => [profile.id, profile]));
  const selected = lane.escalation
    .filter((profileId) => profileId !== currentProfileId)
    .map((profileId) => profiles.get(profileId))
    .find((profile): profile is CapabilityProfile =>
      profile === undefined ? false : meetsMinimum(profile, lane.minimum),
    );
  if (!selected) {
    const current = profiles.get(currentProfileId);
    if (!current) throw new Error(`unknown current routing profile ${currentProfileId}`);
    return routeDecision(
      laneId,
      current,
      [],
      `No stronger eligible escalation follows ${current.id}; retaining the current route.`,
      false,
    );
  }
  const alternatives = guide.profiles.filter((profile) => profile.id !== selected.id);
  return routeDecision(
    laneId,
    selected,
    alternatives,
    `Escalated to ${selected.id} after a capability-classified rejection.`,
    false,
  );
}

function routeDecision(
  lane: RouteLaneId,
  selected: CapabilityProfile,
  alternatives: CapabilityProfile[],
  reason: string,
  escalationAllowed: boolean,
): RouteDecision {
  return {
    lane,
    profileId: selected.id,
    model: selected.model,
    reasoningEffort: selected.reasoningEffort,
    reason,
    alternatives: alternatives.map((profile) => ({
      profileId: profile.id,
      model: profile.model,
      reasoningEffort: profile.reasoningEffort,
      rejectedBecause: "Lower lane affinity, higher cost rank, or reserved for escalation.",
    })),
    escalationAllowed,
  };
}

function meetsMinimum(profile: CapabilityProfile, minimum: Record<string, number>): boolean {
  return Object.entries(minimum).every(
    ([capability, required]) => (profile.capabilities[capability] ?? 0) >= required,
  );
}

function affinityRank(lane: RoutingLane, profileId: string): number {
  const rank = lane.affinity.findIndex((group) => group.includes(profileId));
  return rank === -1 ? Number.MAX_SAFE_INTEGER : rank;
}

function routeLaneIds(): RouteLaneId[] {
  return ["architecture", "implementation", "journeyReview", "codeQualityReview"];
}

function writableAttemptsForPhase(
  attempts: Array<Pick<SessionResult, "phaseId" | "role">>,
  phaseId: string,
): number {
  return attempts.filter(
    (attempt) =>
      attempt.phaseId === phaseId &&
      (attempt.role === "implementation" || attempt.role === "repair"),
  ).length;
}

function phaseFailure(phaseId: string, sessionIds: string[], handoff: string): PhaseResult {
  return {
    phaseId,
    accepted: false,
    sessionIds,
    handoff: handoff.slice(0, MAX_HANDOFF_CHARS),
    projection: null,
  };
}

function implementationProjection(output: ImplementationOutput): ImplementationOutput {
  return {
    status: output.status,
    summary: output.summary.slice(0, 2_000),
    decisions: output.decisions.slice(0, 3).map((decision) => ({
      decision: decision.decision.slice(0, 500),
      reason: decision.reason.slice(0, 500),
      alternatives: decision.alternatives
        .slice(0, 3)
        .map((alternative) => alternative.slice(0, 300)),
    })),
    verification: output.verification.slice(0, 3).map((item) => item.slice(0, 500)),
    blockingIssue: output.blockingIssue?.slice(0, 1_000) ?? null,
    handoff: output.handoff.slice(0, 1_500),
  };
}

function reviewProjection(output: ReviewOutput): ReviewOutput {
  return {
    decision: output.decision,
    failureClass: output.failureClass,
    summary: output.summary.slice(0, 2_000),
    findings: output.findings.slice(0, 4).map((finding) => ({
      severity: finding.severity,
      problem: finding.problem.slice(0, 800),
      evidence: finding.evidence.slice(0, 800),
      requiredChange: finding.requiredChange.slice(0, 800),
    })),
  };
}

function isImplementationRepair(output: ReviewOutput): boolean {
  return (
    output.decision === "needs_repair" &&
    (output.failureClass === "capability" || output.failureClass === "implementation_defect")
  );
}

function limitsFromOptions(options: RuntimeOptions): WorkflowLimits {
  return {
    maxSessions: options.maxSessions,
    maxRepairsPerPhase: options.maxRepairsPerPhase,
    maxSessionMs: options.maxSessionMs,
    maxRunMs: options.maxRunMs,
  };
}

function loadState(
  stateFile: string,
): (Partial<WorkflowState> & Pick<WorkflowState, "runId" | "repoRoot" | "startedAt">) | null {
  return fs.existsSync(stateFile)
    ? (JSON.parse(fs.readFileSync(stateFile, "utf8")) as Partial<WorkflowState> &
        Pick<WorkflowState, "runId" | "repoRoot" | "startedAt">)
    : null;
}

function trySnapshotGit(repoRoot: string): GitSnapshot | null {
  try {
    return snapshotGit(repoRoot);
  } catch {
    return null;
  }
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function routingHash(routing: RoutingGuide): string {
  return createHash("sha256").update(JSON.stringify(routing)).digest("hex");
}

function phaseDefinitionHash(phase: PhaseDefinition): string {
  return createHash("sha256").update(JSON.stringify(phase)).digest("hex");
}

function sameStringArray(left: string[], right: string[]): boolean {
  return left.length === right.length && left.every((value, index) => value === right[index]);
}

function definedEnvironment(): Record<string, string> {
  return Object.fromEntries(
    Object.entries(process.env).filter(
      (entry): entry is [string, string] => entry[1] !== undefined,
    ),
  );
}

import { z } from "zod";
import { zodToJsonSchema } from "zod-to-json-schema";

const decisionSchema = z.object({
  decision: z.string().max(2_000),
  reason: z.string().max(2_000),
  alternatives: z.array(z.string().max(1_000)).max(8),
});

export const implementationOutputSchema = z.object({
  status: z.enum(["completed", "blocked", "ask_user"]),
  summary: z.string().max(4_000),
  decisions: z.array(decisionSchema).max(12),
  verification: z.array(z.string().max(2_000)).max(12),
  blockingIssue: z.string().max(4_000).nullable(),
  handoff: z.string().max(12_000),
});

export const reviewOutputSchema = z
  .object({
    decision: z.enum(["accepted", "needs_repair", "blocked"]),
    failureClass: z.enum(["none", "capability", "implementation_defect", "environment", "scope"]),
    summary: z.string().max(4_000),
    findings: z
      .array(
        z.object({
          severity: z.enum(["low", "medium", "high"]),
          problem: z.string().max(2_000),
          evidence: z.string().max(2_000),
          requiredChange: z.string().max(2_000),
        }),
      )
      .max(20),
  })
  .superRefine((output, context) => {
    const acceptedWithFailure = output.decision === "accepted" && output.failureClass !== "none";
    const rejectedWithoutFailure = output.decision !== "accepted" && output.failureClass === "none";
    if (acceptedWithFailure || rejectedWithoutFailure) {
      context.addIssue({
        code: z.ZodIssueCode.custom,
        path: ["failureClass"],
        message: "accepted requires none; every rejection requires a concrete failure class",
      });
    }
  });

export type ImplementationOutput = z.infer<typeof implementationOutputSchema>;
export type ReviewOutput = z.infer<typeof reviewOutputSchema>;

export function jsonSchemaFor(schema: z.ZodTypeAny): Record<string, unknown> {
  return zodToJsonSchema(schema, { target: "openAi" }) as Record<string, unknown>;
}

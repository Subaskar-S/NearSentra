import { z } from "zod";

/** NEAR account ID grammar (lowercase alphanumerics with `-`/`_` separators and `.` sub-accounts). */
export const NEAR_ACCOUNT_REGEX = /^(([a-z\d]+[-_])*[a-z\d]+\.)*([a-z\d]+[-_])*[a-z\d]+$/;

export const U128_MAX = 2n ** 128n - 1n;

const accountId = (label: string) =>
  z
    .string({
      required_error: `${label} is required`,
      invalid_type_error: `${label} must be a string`,
    })
    .min(2, `${label} must be at least 2 characters`)
    .max(64, `${label} must be at most 64 characters`)
    .regex(NEAR_ACCOUNT_REGEX, `${label} must be a valid NEAR account ID`);

/**
 * Amount in yoctoNEAR as a canonical base-10 integer string (no sign, no leading
 * zeros, no decimals). Strings keep full precision; JSON numbers are rejected.
 */
const AMOUNT_FORMAT = /^(0|[1-9]\d{0,38})$/;

const amount = z
  .string({
    required_error: "amount is required",
    invalid_type_error: "amount must be a string of digits (yoctoNEAR)",
  })
  .regex(AMOUNT_FORMAT, "amount must be a base-10 integer string without sign or leading zeros")
  .refine((v) => {
    // Zod still runs refinements after a failed regex, so guard before BigInt().
    if (!AMOUNT_FORMAT.test(v)) return true; // format error already reported
    const n = BigInt(v);
    return n > 0n && n <= U128_MAX;
  }, "amount must be greater than 0 and at most 2^128 - 1");

/**
 * Request body for POST /transfer. Every field is untrusted (agent- or LLM-provided).
 * `.strict()` rejects unexpected keys so nothing unvalidated is ever forwarded.
 */
export const transferRequestSchema = z
  .object({
    agent_id: accountId("agent_id"),
    recipient: accountId("recipient"),
    amount,
    request_id: z
      .string({
        required_error: "request_id is required",
        invalid_type_error: "request_id must be a string",
      })
      .min(1, "request_id must not be empty")
      .max(128, "request_id must be at most 128 characters"),
  })
  .strict();

export type TransferRequest = z.infer<typeof transferRequestSchema>;

export interface FieldIssue {
  field: string;
  constraint: string;
}

/** Flatten a ZodError into `{ field, constraint }` entries, one per violation. */
export function toFieldIssues(error: z.ZodError): FieldIssue[] {
  return error.issues.flatMap((issue): FieldIssue[] => {
    if (issue.code === "unrecognized_keys") {
      return issue.keys.map((k) => ({ field: k, constraint: "unrecognized field" }));
    }
    return [{ field: issue.path.join(".") || "(body)", constraint: issue.message }];
  });
}

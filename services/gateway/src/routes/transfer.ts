import type { FastifyBaseLogger, FastifyInstance } from "fastify";
import { toFieldIssues, transferRequestSchema } from "../schemas/transfer.js";
import type { VaultClient, PolicyView } from "../near/client.js";
import { IdempotencyStore, type StoredResponse } from "../idempotency.js";

export interface TransferRouteDeps {
  vault: VaultClient;
  /** Defaults to a fresh in-memory store. Injectable for tests. */
  idempotency?: IdempotencyStore;
}

/**
 * Advisory-only observations about the retrieved policy. They are logged for
 * operators but NEVER used to approve or deny: the vault contract is the final
 * authority, so every validated request is forwarded.
 */
function advisoryNotes(policy: PolicyView | null, recipient: string, amount: string): string[] {
  if (!policy) return ["agent not registered on vault"];
  const notes: string[] = [];
  if (policy.revoked) notes.push("policy is revoked");
  else if (!policy.active) notes.push("agent is paused");
  if (!policy.allowlist.includes(recipient)) notes.push("recipient not on allowlist");
  if (BigInt(amount) > BigInt(policy.per_tx_limit)) notes.push("amount exceeds per_tx_limit");
  if (BigInt(amount) > BigInt(policy.remaining_epoch_budget)) notes.push("amount exceeds remaining epoch budget");
  return notes;
}

interface Processed {
  response: StoredResponse;
  /**
   * True once a transaction may have reached the chain. Such results are remembered so a
   * retry with the same request_id cannot pay twice. Results where nothing was submitted
   * (preflight failure, no key) are not remembered, so the agent can simply retry.
   */
  cache: boolean;
}

export function registerTransferRoute(
  app: FastifyInstance,
  { vault, idempotency = new IdempotencyStore() }: TransferRouteDeps,
): void {
  async function process(
    log: FastifyBaseLogger,
    agentId: string,
    recipient: string,
    amount: string,
    requestId: string,
  ): Promise<Processed> {
    // 2. Preflight view call (informational). RPC failure => 503, nothing submitted.
    let policy: PolicyView | null;
    try {
      policy = await vault.viewAgentPolicy(agentId);
    } catch (err) {
      log.error({ err: (err as Error).message }, "preflight view call failed");
      return {
        cache: false,
        response: {
          status: 503,
          body: {
            error: "preflight_failed",
            message: "could not read agent policy from the vault; no transaction was submitted",
          },
        },
      };
    }
    const notes = advisoryNotes(policy, recipient, amount);
    log.info(
      { policy, advisory: notes },
      notes.length
        ? "preflight: advisory concerns noted; forwarding to contract, which makes the final decision"
        : "preflight: no concerns noted; forwarding to contract, which makes the final decision",
    );

    // 3. The gateway can only sign for agents it holds a scoped key for.
    if (!vault.hasAgentKey(agentId)) {
      log.warn("no signing key configured for agent");
      return {
        cache: false,
        response: {
          status: 422,
          body: {
            error: "agent_key_not_configured",
            message: "this gateway holds no request_transfer key for the given agent_id",
          },
        },
      };
    }

    // 4. Submit. Network / nonce / RPC problems => 502, never a fabricated outcome.
    // From here on a transaction may have been broadcast, so the result is remembered.
    let outcome;
    try {
      outcome = await vault.submitTransfer(agentId, recipient, amount);
    } catch (err) {
      log.error({ err: (err as Error).message }, "transaction submission failed");
      return {
        cache: true,
        response: {
          status: 502,
          body: {
            error: "submission_failed",
            message:
              "the transaction could not be submitted or its outcome could not be retrieved; " +
              "it may still have executed, so check the chain and use a new request_id to retry on purpose",
          },
        },
      };
    }

    if (outcome.status === "unknown") {
      log.error({ txHash: outcome.txHash }, "transaction outcome not final");
      return {
        cache: true,
        response: {
          status: 502,
          body: {
            error: "outcome_unknown",
            txHash: outcome.txHash,
            message: "the transaction outcome is not available yet; check the hash on-chain",
          },
        },
      };
    }

    // 5. Report exactly what the chain decided. Policy violations are HTTP 200.
    const approved = outcome.status === "success";
    const body = {
      requestId,
      txHash: outcome.txHash,
      decision: approved ? ("approved" as const) : ("rejected" as const),
      rejectionReason: approved ? null : (outcome.rejectionReason ?? outcome.failureMessage),
      rolledBack: outcome.rolledBack,
      preflightPolicy: policy,
    };
    log.info(
      { txHash: body.txHash, decision: body.decision, rejectionReason: body.rejectionReason },
      "transfer processed",
    );
    return { cache: true, response: { status: 200, body } };
  }

  app.post("/transfer", async (request, reply) => {
    // 1. Validate everything (untrusted input) before any other processing.
    const parsed = transferRequestSchema.safeParse(request.body);
    if (!parsed.success) {
      const issues = toFieldIssues(parsed.error);
      request.log.warn({ issues }, "transfer request failed validation");
      return reply.code(400).send({ error: "validation_failed", issues });
    }
    const { agent_id: agentId, recipient, amount, request_id: requestId } = parsed.data;
    const log = request.log.child({ agentId, recipient, amount, requestId });

    // 1b. Idempotency: the same (agent_id, request_id) must never pay twice.
    const claim = idempotency.claim(
      IdempotencyStore.key(agentId, requestId),
      IdempotencyStore.fingerprint(recipient, amount),
    );

    if (claim.kind === "conflict") {
      log.warn("request_id reused with a different recipient or amount");
      return reply.code(409).send({
        error: "request_id_conflict",
        message: "this request_id was already used for a different recipient or amount",
      });
    }
    if (claim.kind === "full") {
      log.error("idempotency store is full of in-flight requests");
      return reply.code(503).send({
        error: "too_many_pending",
        message: "too many transfers are in flight; retry shortly",
      });
    }
    if (claim.kind === "replay") {
      log.info("duplicate request_id: returning the original response, nothing resubmitted");
      const original = await claim.promise;
      return reply.code(original.status).header("idempotent-replay", "true").send(original.body);
    }

    try {
      const { response, cache } = await process(log, agentId, recipient, amount, requestId);
      claim.complete(response, cache);
      return reply.code(response.status).send(response.body);
    } catch (err) {
      claim.fail(err);
      throw err;
    }
  });
}

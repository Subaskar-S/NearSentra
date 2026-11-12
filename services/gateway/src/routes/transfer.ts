import type { FastifyInstance } from "fastify";
import { toFieldIssues, transferRequestSchema } from "../schemas/transfer.js";
import type { VaultClient, PolicyView } from "../near/client.js";

export interface TransferRouteDeps {
  vault: VaultClient;
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

export function registerTransferRoute(app: FastifyInstance, { vault }: TransferRouteDeps): void {
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

    // 2. Preflight view call (informational). RPC failure => 503, nothing submitted.
    let policy: PolicyView | null;
    try {
      policy = await vault.viewAgentPolicy(agentId);
    } catch (err) {
      log.error({ err: (err as Error).message }, "preflight view call failed");
      return reply.code(503).send({
        error: "preflight_failed",
        message: "could not read agent policy from the vault; no transaction was submitted",
      });
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
      return reply.code(422).send({
        error: "agent_key_not_configured",
        message: "this gateway holds no request_transfer key for the given agent_id",
      });
    }

    // 4. Submit. Network / nonce / RPC problems => 502, never a fabricated outcome.
    let outcome;
    try {
      outcome = await vault.submitTransfer(agentId, recipient, amount);
    } catch (err) {
      log.error({ err: (err as Error).message }, "transaction submission failed");
      return reply.code(502).send({
        error: "submission_failed",
        message: "the transaction could not be submitted or its outcome could not be retrieved",
      });
    }

    if (outcome.status === "unknown") {
      log.error({ txHash: outcome.txHash }, "transaction outcome not final");
      return reply.code(502).send({
        error: "outcome_unknown",
        txHash: outcome.txHash,
        message: "the transaction outcome is not available yet; check the hash on-chain",
      });
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
    return reply.code(200).send(body);
  });
}

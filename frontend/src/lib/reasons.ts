/** Plain-language titles for the vault's `ERR_*` rejection codes (see contracts/vault/src/errors.rs). */
const TITLES: Record<string, string> = {
  ERR_PER_TX_LIMIT: 'Over the per-transaction limit',
  ERR_EPOCH_LIMIT: 'Over the epoch budget',
  ERR_RECIPIENT_NOT_ALLOWED: 'Recipient is not on the allowlist',
  ERR_AGENT_NOT_REGISTERED: 'Caller is not a registered agent',
  ERR_AGENT_PAUSED: 'Agent is paused',
  ERR_AGENT_REVOKED: 'Agent is permanently revoked',
  ERR_VAULT_PAUSED: 'Vault is paused',
  ERR_POLICY_EXPIRED: 'Agent policy has expired',
  ERR_AMOUNT_ZERO: 'Amount must be greater than zero',
  ERR_TRANSFER_PENDING: 'Previous transfer is still settling',
  ERR_OVERFLOW: 'Arithmetic overflow',
};

const CODE = /ERR_[A-Z0-9_]+/;

export interface Reason {
  /** Readable sentence, or the vault's own message when the code is not known here. */
  title: string;
  /** The contract's `ERR_*` code, or null when the text has none. */
  code: string | null;
}

/** Turn the vault's `ERR_X: message` string into a readable title. Never hides the original code. */
export function describeReason(raw: string | null): Reason | null {
  if (!raw) return null;
  const code = CODE.exec(raw)?.[0] ?? null;
  if (code && TITLES[code]) return { title: TITLES[code], code };
  return { title: raw.replace(/^ERR_[A-Z0-9_]+:\s*/, ''), code };
}

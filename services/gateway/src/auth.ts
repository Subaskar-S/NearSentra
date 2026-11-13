import { createHash, timingSafeEqual } from "node:crypto";

export interface AuthConfig {
  /** agent account ID -> bearer token. An agent with a token always requires it. */
  tokens: Map<string, string>;
  /** Serve agents that have no token without authentication (loopback-only local development). */
  allowUnauthenticated: boolean;
}

const digest = (value: string): Buffer => createHash("sha256").update(value).digest();

/** Extract the token from an `Authorization: Bearer <token>` header, or null if it is not one. */
export function parseBearer(header: unknown): string | null {
  if (typeof header !== "string") return null;
  const match = /^Bearer (\S+)$/i.exec(header.trim());
  return match ? match[1]! : null;
}

/**
 * Whether the caller may act for `agentId`. The token is compared in constant time (on SHA-256
 * digests, so the lengths always match). An agent without a configured token is only served when
 * `allowUnauthenticated` is set, which also keeps unknown agent IDs indistinguishable from wrong tokens.
 */
export function isAuthorized(auth: AuthConfig, agentId: string, header: unknown): boolean {
  const expected = auth.tokens.get(agentId);
  if (expected === undefined) return auth.allowUnauthenticated;
  const presented = parseBearer(header);
  if (presented === null) return false;
  return timingSafeEqual(digest(presented), digest(expected));
}

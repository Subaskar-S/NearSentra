import { z } from "zod";

const privateKey = z.string().regex(/^ed25519:[1-9A-HJ-NP-Za-km-z]{40,100}$/, "must be an ed25519 key");

const envSchema = z.object({
  NEAR_RPC_URL: z.string().url(),
  VAULT_ACCOUNT_ID: z.string().min(2).max(64),
  HOST: z.string().default("127.0.0.1"),
  PORT: z.coerce.number().int().min(1).max(65535).default(3000),
  LOG_LEVEL: z.enum(["fatal", "error", "warn", "info", "debug", "trace", "silent"]).default("info"),
  GATEWAY_ALLOW_UNAUTHENTICATED: z.enum(["true", "false"]).default("false"),
});

const LOOPBACK_HOSTS = new Set(["127.0.0.1", "localhost", "::1"]);
const MIN_TOKEN_LENGTH = 32;

export interface GatewayConfig {
  rpcUrl: string;
  vaultAccountId: string;
  host: string;
  port: number;
  logLevel: string;
  /** agent account ID -> function-call private key (scoped to `request_transfer`). */
  agentKeys: Map<string, string>;
  /** agent account ID -> bearer token that clients must present to spend that agent's budget. */
  agentTokens: Map<string, string>;
  /** Serve agents that have no token without authentication (loopback-only local development). */
  allowUnauthenticated: boolean;
}

/**
 * Load configuration from environment variables only. Agent keys are provided as
 * numbered pairs: AGENT_<n>_ACCOUNT_ID / AGENT_<n>_PRIVATE_KEY. Keys must be
 * function-call keys restricted to `request_transfer` on the vault. Each agent also needs
 * AGENT_<n>_API_TOKEN (at least 32 characters, unique per agent) unless
 * GATEWAY_ALLOW_UNAUTHENTICATED=true, which is refused for any non-loopback HOST.
 * Throws a descriptive error (never echoing key or token material) on invalid config.
 */
export function loadConfig(env: NodeJS.ProcessEnv = process.env): GatewayConfig {
  const parsed = envSchema.safeParse(env);
  if (!parsed.success) {
    const problems = parsed.error.issues.map((i) => `${i.path.join(".")}: ${i.message}`).join("; ");
    throw new Error(`Invalid gateway configuration: ${problems}`);
  }

  const agentKeys = new Map<string, string>();
  for (const name of Object.keys(env)) {
    const m = /^AGENT_(\d+)_ACCOUNT_ID$/.exec(name);
    if (!m) continue;
    const accountId = env[name];
    const key = env[`AGENT_${m[1]}_PRIVATE_KEY`];
    if (!accountId || !key) {
      throw new Error(`AGENT_${m[1]}_ACCOUNT_ID is set but AGENT_${m[1]}_PRIVATE_KEY is missing`);
    }
    if (!privateKey.safeParse(key).success) {
      // Do not include the value in the message.
      throw new Error(`AGENT_${m[1]}_PRIVATE_KEY is not a valid ed25519 key`);
    }
    agentKeys.set(accountId, key);
  }

  const allowUnauthenticated = parsed.data.GATEWAY_ALLOW_UNAUTHENTICATED === "true";
  if (allowUnauthenticated && !LOOPBACK_HOSTS.has(parsed.data.HOST)) {
    throw new Error("GATEWAY_ALLOW_UNAUTHENTICATED=true is only allowed when HOST is a loopback address");
  }

  const agentTokens = new Map<string, string>();
  const tokenOwner = new Map<string, string>();
  for (const name of Object.keys(env)) {
    const m = /^AGENT_(\d+)_API_TOKEN$/.exec(name);
    if (!m) continue;
    const accountId = env[`AGENT_${m[1]}_ACCOUNT_ID`];
    const token = env[name];
    if (!accountId) throw new Error(`AGENT_${m[1]}_API_TOKEN is set but AGENT_${m[1]}_ACCOUNT_ID is missing`);
    if (!token || token.length < MIN_TOKEN_LENGTH || /\s/.test(token)) {
      // Do not include the value in the message.
      throw new Error(`AGENT_${m[1]}_API_TOKEN must be at least ${MIN_TOKEN_LENGTH} characters without whitespace`);
    }
    if (tokenOwner.has(token)) {
      throw new Error(`AGENT_${m[1]}_API_TOKEN duplicates another agent's token; every agent needs its own`);
    }
    tokenOwner.set(token, accountId);
    agentTokens.set(accountId, token);
  }
  if (!allowUnauthenticated) {
    for (const accountId of agentKeys.keys()) {
      if (!agentTokens.has(accountId)) {
        throw new Error(
          `no AGENT_<n>_API_TOKEN configured for ${accountId}; set one, or GATEWAY_ALLOW_UNAUTHENTICATED=true ` +
            "for a loopback-only local sandbox",
        );
      }
    }
  }

  return {
    rpcUrl: parsed.data.NEAR_RPC_URL,
    vaultAccountId: parsed.data.VAULT_ACCOUNT_ID,
    host: parsed.data.HOST,
    port: parsed.data.PORT,
    logLevel: parsed.data.LOG_LEVEL,
    agentKeys,
    agentTokens,
    allowUnauthenticated,
  };
}

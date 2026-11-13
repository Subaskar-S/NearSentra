import { buildApp } from "./app.js";
import { loadConfig } from "./config.js";
import { NearVaultClient } from "./near/client.js";

async function main(): Promise<void> {
  const config = loadConfig();
  const vault = new NearVaultClient(config.vaultAccountId, config.rpcUrl, config.agentKeys);
  const auth = { tokens: config.agentTokens, allowUnauthenticated: config.allowUnauthenticated };
  const app = await buildApp({ vault, auth, logLevel: config.logLevel });
  if (config.allowUnauthenticated) {
    app.log.warn("GATEWAY_ALLOW_UNAUTHENTICATED=true: agents without a token are served without authentication");
  }
  app.log.info(
    { vault: config.vaultAccountId, rpc: config.rpcUrl, agents: [...config.agentKeys.keys()] },
    "gateway starting",
  );
  await app.listen({ host: config.host, port: config.port });
}

main().catch((err: unknown) => {
  // eslint-disable-next-line no-console
  console.error(err instanceof Error ? err.message : String(err));
  process.exit(1);
});

import { buildApp } from "./app.js";
import { loadConfig } from "./config.js";
import { NearVaultClient } from "./near/client.js";

async function main(): Promise<void> {
  const config = loadConfig();
  const vault = new NearVaultClient(config.vaultAccountId, config.rpcUrl, config.agentKeys);
  const app = await buildApp({ vault, logLevel: config.logLevel });
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

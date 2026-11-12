import Fastify, { type FastifyInstance } from "fastify";
import sensible from "@fastify/sensible";
import type { VaultClient } from "./near/client.js";
import { registerTransferRoute } from "./routes/transfer.js";

export interface BuildAppOptions {
  vault: VaultClient;
  logLevel?: string;
}

/** Build the Fastify app. Separate from `index.ts` so tests can inject a fake vault. */
export async function buildApp({ vault, logLevel = "info" }: BuildAppOptions): Promise<FastifyInstance> {
  const app = Fastify({
    bodyLimit: 16 * 1024,
    logger: {
      level: logLevel,
      // Defence in depth: never let key material or auth headers reach a log line.
      redact: {
        paths: ["req.headers.authorization", "*.privateKey", "*.private_key", "*.secretKey", "*.seedPhrase"],
        censor: "[redacted]",
      },
    },
  });
  await app.register(sensible);

  app.get("/health", async () => ({ status: "ok" }));
  registerTransferRoute(app, { vault });
  return app;
}

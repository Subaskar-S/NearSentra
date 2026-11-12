// NearSentra attack-simulation runner. Run via `make simulate` (scripts/run-simulation.sh),
// which starts a throwaway local network and the Sentinel and exports the environment.
import { loadContext } from "./helpers.js";
import { run as scenario1 } from "./scenarios/scenario1_normal.js";
import { run as scenario2 } from "./scenarios/scenario2_rogue.js";
import { run as scenario3 } from "./scenarios/scenario3_runaway.js";
import { run as scenario4 } from "./scenarios/scenario4_sentinel.js";
import { run as scenario5 } from "./scenarios/scenario5_revoked.js";

export { assertOutcome, printScenarioResult } from "./helpers.js";

async function main(): Promise<void> {
  const ctx = loadContext();
  const scenarios = [scenario1, scenario2, scenario3, scenario4, scenario5];
  for (const [i, scenario] of scenarios.entries()) {
    console.log(`\n=== Scenario ${i + 1} of ${scenarios.length} ===`);
    await scenario(ctx);
  }
  console.log(`\nAll ${scenarios.length} scenarios produced the expected on-chain outcome.`);
}

main().catch((err: unknown) => {
  console.error("Simulation aborted:", err instanceof Error ? err.message : err);
  process.exit(1);
});

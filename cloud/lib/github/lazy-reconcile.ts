import "server-only";
import type { Db } from "@/lib/db/types";
import { githubAppConfig } from "./env";
import { reconcileTeamIfDue } from "./reconcile";
import { githubForReconcile } from "./server";

// The live poller's hook (lib/github/live.ts): while someone watches a team,
// reconcile it at most every 15 minutes (the reconcile_state claim makes that
// hold across instances). Never throws; never awaited by the stream.

export async function lazyReconcileTeam(teamId: string, db: Db): Promise<void> {
  const cfg = githubAppConfig();
  if (!cfg.ok) return;
  try {
    const result = await reconcileTeamIfDue(db, githubForReconcile(cfg.config), teamId);
    if (result) console.info("[quod-cloud] github reconcile (lazy)", { team: teamId, ...result });
  } catch (e) {
    console.warn("[quod-cloud] github reconcile (lazy) failed", { team: teamId, error: e instanceof Error ? e.name : "unknown" });
  }
}

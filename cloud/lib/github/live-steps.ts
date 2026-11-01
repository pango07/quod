import "server-only";
import type { Db } from "@/lib/db/types";
import { appJwt, getJob, installationTokenWithExpiry } from "./api";
import { githubAppConfig, type GitHubAppConfig } from "./env";
import { pollTeamSteps, type StepPollGitHub } from "./step-poll";

// The live poller's step hook (lib/github/live.ts): while someone watches a
// team, read its running jobs' steps from GitHub (lib/github/step-poll.ts).
// Never throws; never awaited by the stream.

// Installation tokens live an hour; one per installation per instance,
// in memory only (never stored, never logged), renewed 5 minutes early.
const tokens = new Map<number, { token: Promise<string>; expiresAt: number }>();

function tokenFor(config: GitHubAppConfig, installationId: number): Promise<string> {
  const hit = tokens.get(installationId);
  if (hit && hit.expiresAt - 5 * 60_000 > Date.now()) return hit.token;
  const entry = { token: Promise.resolve(""), expiresAt: Date.now() + 10 * 60_000 };
  entry.token = installationTokenWithExpiry(appJwt(config.appId, config.privateKey), installationId).then(
    (t) => ((entry.expiresAt = t.expiresAt), t.token),
    (e) => {
      tokens.delete(installationId); // the next pass asks again
      throw e;
    },
  );
  tokens.set(installationId, entry);
  return entry.token;
}

export function githubForSteps(config: GitHubAppConfig): StepPollGitHub {
  return { getJob: async (installationId, fullName, jobId) => getJob(await tokenFor(config, installationId), fullName, jobId) };
}

export async function pollTeamStepsLive(teamId: string, db: Db): Promise<void> {
  const cfg = githubAppConfig();
  if (!cfg.ok) return;
  try {
    const r = await pollTeamSteps(db, githubForSteps(cfg.config), teamId);
    if (r.limited || r.errors) console.warn("[quod-cloud] github step poll", { team: teamId, ...r });
  } catch (e) {
    console.warn("[quod-cloud] github step poll failed", { team: teamId, error: e instanceof Error ? e.name : "unknown" });
  }
}

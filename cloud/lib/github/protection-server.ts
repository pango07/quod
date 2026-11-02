import "server-only";
import { waitUntil } from "@vercel/functions";
import type { BranchProtection } from "@/lib/api/types";
import type { Db } from "@/lib/db/types";
import * as gh from "./api";
import { githubAppConfig } from "./env";
import { readCached, refreshProtection, type ProtectionGitHub } from "./protection";
import type { TeamId } from "./team";

// The pages' entry to branch protection: the cache now, and a background
// refresh of what is due (never awaited by the page; Vercel's waitUntil keeps
// the function alive for it). Without the GitHub App's credentials nothing is
// read and every repo stays "unknown".

function protectionGitHub(): ProtectionGitHub | null {
  const cfg = githubAppConfig();
  if (!cfg.ok) return null;
  const { appId, privateKey } = cfg.config;
  const tokens = new Map<number, Promise<string>>();
  return {
    read: async (installationId, fullName) => {
      let t = tokens.get(installationId);
      if (!t) tokens.set(installationId, (t = gh.installationToken(gh.appJwt(appId, privateKey), installationId)));
      return gh.readDefaultBranchProtection(await t, fullName);
    },
  };
}

export async function protectionFor(teamId: TeamId, db: Db, repos: { id: number }[]): Promise<Map<number, BranchProtection> | null> {
  const ids = repos.map((r) => r.id);
  const { byRepo, due } = await readCached(db, teamId, ids);
  const client = due.length ? protectionGitHub() : null;
  if (client) {
    waitUntil(
      refreshProtection(db, teamId, due, client).then((r) => {
        if (r.read || r.limited) console.info("[quod-cloud] branch protection refreshed", { team: teamId, ...r });
      }),
    );
  }
  return byRepo;
}

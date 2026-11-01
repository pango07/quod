"use server";

import { redirect } from "next/navigation";
import { getViewer } from "@/lib/auth/session";
import { isAdmin } from "@/lib/auth/types";
import { getTeams } from "@/lib/dal";
import { getDb } from "@/lib/db";
import { githubAppConfig } from "./env";
import { appInstallationRepos, clearClaim, readClaim } from "./server";
import { linkInstallation } from "./store";

// The confirm step of linking. The claim cookie holds what /github/installed
// verified with GitHub; this re-checks the parts that can change in the ten
// minutes it lives (the viewer, their membership and role, the installation
// not taken by another team) and links in one transaction.

export async function confirmLink(): Promise<void> {
  const claim = await readClaim();
  if (!claim) redirect("/github/link?error=expired");
  const [viewer, teams] = await Promise.all([getViewer(), getTeams()]);
  if (viewer.id !== claim.userId) redirect("/github/link?error=expired");
  const team = teams.find((t) => t.orgId === claim.teamOrgId);
  if (!team) redirect("/github/link?error=not-member");
  if (!isAdmin(team.role)) redirect("/github/link?error=not-admin");

  const cfg = githubAppConfig();
  if (!cfg.ok) redirect("/github/link?error=not-configured");

  let outcome: Awaited<ReturnType<typeof linkInstallation>> | "too-many-repos";
  try {
    const repos = await appInstallationRepos(cfg.config, claim.installationId);
    const db = await getDb();
    outcome = !repos.complete ? "too-many-repos" : await db.tx((q) =>
      linkInstallation(q, {
        installation: { id: claim.installationId, account: claim.account, repository_selection: claim.repositorySelection },
        teamId: team.orgId,
        userId: viewer.id,
        githubUserId: claim.githubUserId,
        repos: repos.items.map((r) => ({ id: r.id, name: r.name, full_name: r.full_name, private: r.private })),
      }),
    );
  } catch (e) {
    console.warn("[quod-cloud] github link failed", { installationId: claim.installationId, error: e instanceof Error ? e.name : "unknown" });
    redirect("/github/link?error=github");
  }
  await clearClaim();
  if (outcome === "too-many-repos") redirect("/github/link?error=too-many-repos");
  if (outcome === "linked-elsewhere") redirect("/github/link?error=linked-elsewhere");
  if (outcome === "deleted") redirect("/github/link?error=uninstalled");
  console.info("[quod-cloud] github installation linked", { installationId: claim.installationId, team: team.orgId });
  redirect(`/github/link?status=linked&team=${encodeURIComponent(team.slug)}&installation=${claim.installationId}`);
}

export async function cancelLink(): Promise<void> {
  await clearClaim();
  redirect("/");
}

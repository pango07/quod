"use server";

import { revalidatePath } from "next/cache";
import { redirect } from "next/navigation";
import { getViewer } from "@/lib/auth/session";
import { isAdmin } from "@/lib/auth/types";
import { getTeams, requireTeam } from "@/lib/dal";
import { getDb } from "@/lib/db";
import { isSlug } from "@/lib/params";
import { githubAppConfig } from "./env";
import { repoSetUnchanged } from "./link";
import { appInstallationRepos, clearClaim, readClaim } from "./server";
import { disconnectInstallation, linkInstallation } from "./store";

// The confirm step of linking, and Disconnect. The claim cookie holds what
// /github/installed verified with GitHub; confirm re-checks what can change
// in the ten minutes it lives (the viewer, their membership and role, the
// installation not taken by another team, the repo set) and links in one
// transaction.

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

  let outcome: Awaited<ReturnType<typeof linkInstallation>> | "too-many-repos" | "changed";
  try {
    const repos = await appInstallationRepos(cfg.config, claim.installationId);
    if (!repos.complete) outcome = "too-many-repos";
    else if (!repoSetUnchanged(claim, repos.items.map((r) => r.id))) outcome = "changed";
    else {
      const db = await getDb();
      outcome = await db.tx((q) =>
        linkInstallation(q, {
          installation: { id: claim.installationId, account: claim.account, repository_selection: claim.repositorySelection },
          teamId: team.orgId,
          userId: viewer.id,
          githubUserId: claim.githubUserId,
          repos: repos.items.map((r) => ({ id: r.id, name: r.name, full_name: r.full_name, private: r.private })),
        }),
      );
    }
  } catch (e) {
    console.warn("[quod-cloud] github link failed", { installationId: claim.installationId, ...errorInfo(e) });
    redirect("/github/link?error=github");
  }
  await clearClaim();
  if (outcome !== "linked") redirect(`/github/link?error=${outcome === "deleted" ? "uninstalled" : outcome}`);
  console.info("[quod-cloud] github installation linked", { installationId: claim.installationId, team: team.orgId });
  redirect(`/github/link?status=linked&team=${encodeURIComponent(team.slug)}&installation=${claim.installationId}`);
}

export async function cancelLink(): Promise<void> {
  await clearClaim();
  redirect("/");
}

export type DisconnectResult = { ok: boolean; message: string } | null;

/** Team settings: stop showing an installation's CI in this team. Admins only; the App stays installed on GitHub. */
export async function disconnectGitHub(slug: string, _prev: DisconnectResult, form: FormData): Promise<DisconnectResult> {
  if (!isSlug(slug)) return { ok: false, message: "Unknown team." };
  const { team, admin } = await requireTeam(slug);
  if (!admin) return { ok: false, message: "Only team admins can change this." };
  const raw = String(form.get("installation") ?? "");
  if (!/^\d{1,15}$/.test(raw)) return { ok: false, message: "Choose an installation." };
  try {
    const done = await disconnectInstallation(await getDb(), team.orgId, Number(raw));
    if (!done) return { ok: false, message: "That installation isn't connected to this team." };
  } catch {
    return { ok: false, message: "Quod Cloud can't reach its database right now. Try again in a minute." };
  }
  console.info("[quod-cloud] github installation disconnected", { installationId: Number(raw), team: team.orgId });
  revalidatePath(`/${team.slug}/settings`);
  return { ok: true, message: "Disconnected. To remove the App from GitHub too, uninstall it in GitHub's settings." };
}

/** What failed, for the log: class, status or code, and the message, never a request or a secret. */
function errorInfo(e: unknown): Record<string, unknown> {
  if (!(e instanceof Error)) return { error: "unknown" };
  const x = e as Error & { status?: number; code?: string };
  return { error: e.constructor?.name || e.name, status: x.status, code: x.code, message: e.message.slice(0, 200) };
}

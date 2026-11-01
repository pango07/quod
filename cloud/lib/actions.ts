"use server";

// Server actions. Each one resolves the team through the DAL again, so it
// authorizes itself and never trusts what the client sent.

import { signOut, switchToOrganization } from "@workos-inc/authkit-nextjs";
import { revalidatePath } from "next/cache";
import { redirect } from "next/navigation";
import { api } from "@/lib/api";
import type { Host, Store } from "@/lib/api/types";
import { authMode } from "@/lib/auth/config";
import { adminPortalLink, inviteMember as sendInvite, type PortalIntent } from "@/lib/auth/directory";
import { requireTeam } from "@/lib/dal";
import { isSlug, parseRunNumber } from "@/lib/params";

export type ActionResult = { ok: boolean; message: string } | null;

const NOT_ADMIN = { ok: false, message: "Only team admins can change this." } as const;

export async function switchTeam(slug: string): Promise<void> {
  const { team, viewer } = await requireTeam(slug);
  if (authMode() === "workos" && viewer.currentOrgId !== team.orgId) {
    // Rescopes the WorkOS session (and its access token) to this team's organization.
    // Redirects to AuthKit when the organization requires SSO or MFA.
    await switchToOrganization(team.orgId, { returnTo: `/${team.slug}` });
  }
  redirect(`/${team.slug}`);
}

export async function signOutAction(): Promise<void> {
  if (authMode() === "demo") redirect("/");
  await signOut();
}

const INTENTS: PortalIntent[] = ["sso", "dsync", "domain_verification", "audit_logs"];

/** Opens the WorkOS Admin Portal, where the team's IT admin sets up SSO or directory sync. */
export async function openAdminPortal(slug: string, intent: PortalIntent): Promise<ActionResult> {
  const { team, admin } = await requireTeam(slug);
  if (!admin) return NOT_ADMIN;
  if (!INTENTS.includes(intent)) return { ok: false, message: "Unknown setting." };
  // The return address comes from config, never from request headers.
  const origin = new URL(process.env.NEXT_PUBLIC_WORKOS_REDIRECT_URI ?? "http://localhost:3000").origin;
  const link = await adminPortalLink(team.orgId, intent, `${origin}/${team.slug}/settings`);
  if (!link) return { ok: false, message: "Demo session: the Admin Portal needs WorkOS. Set the WORKOS_* variables to try it." };
  redirect(link);
}

const EMAIL = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

export async function inviteMember(slug: string, _prev: ActionResult, form: FormData): Promise<ActionResult> {
  const { team, admin, viewer } = await requireTeam(slug);
  if (!admin) return NOT_ADMIN;
  const email = String(form.get("email") ?? "").trim();
  const role = form.get("role") === "admin" ? "admin" : "member";
  if (!EMAIL.test(email)) return { ok: false, message: "Enter an email address, like dev@example.com." };
  try {
    const result = await sendInvite(team.orgId, viewer.id, email, role);
    revalidatePath(`/${team.slug}/settings`);
    return result;
  } catch (e) {
    // WorkOS error text can name internals; log it here and tell the user something fixed.
    console.error("[quod-cloud] invitation failed", e);
    return { ok: false, message: "The invitation could not be sent. Check the address, or try again in a minute." };
  }
}

export async function setProofsNeeded(slug: string, repo: string, key: string, need: number): Promise<ActionResult> {
  const { team, admin } = await requireTeam(slug);
  if (!admin) return NOT_ADMIN;
  if (!isSlug(repo) || !isSlug(key)) return { ok: false, message: "No such step." };
  if (![2, 3, 5].includes(need)) return { ok: false, message: "Choose 2, 3 or 5 runs." };
  await api().setProofsNeeded(team.slug, repo, key, need);
  revalidatePath(`/${team.slug}/${repo}/steps/${key}`);
  return { ok: true, message: `Quod now needs ${need} consistent runs.` };
}

/** Any team member may ask for this, as anyone with access can re-run a workflow in GitHub (DECISIONS 2026-11-01). */
export async function rerunWithoutQuod(slug: string, repo: string, run: number): Promise<ActionResult> {
  const { team } = await requireTeam(slug);
  const number = parseRunNumber(run);
  if (!isSlug(repo) || number === null) return { ok: false, message: "No such run." };
  const r = await api().requestRerunWithoutQuod(team.slug, repo, number);
  return { ok: r.accepted, message: r.message };
}

/** Saving the team's setup choice is for admins; members can still look at both setups. */
export async function saveConnectChoice(slug: string, store: Store, host: Host): Promise<ActionResult> {
  const { team, admin } = await requireTeam(slug);
  if (!admin) return NOT_ADMIN;
  if (!["cloud", "self"].includes(store) || !["github", "own"].includes(host)) return { ok: false, message: "Unknown choice." };
  await api().saveConnectChoice(team.slug, { store, host });
  return { ok: true, message: "Saved for the team." };
}

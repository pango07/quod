import { randomBytes } from "node:crypto";
import { sealData, unsealData } from "iron-session";
import { cookies } from "next/headers";
import { connection, NextResponse, type NextRequest } from "next/server";
import { getViewer } from "@/lib/auth/session";
import { getTeams } from "@/lib/dal";
import { getDb } from "@/lib/db";
import { GitHubApiError } from "@/lib/github/api";
import { appOrigin, githubAppConfig } from "@/lib/github/env";
import { verifyInstallationClaim } from "@/lib/github/link";
import { currentTeam, githubForLinking, requestOrigin, saveClaim, sealPassword, secureCookies } from "@/lib/github/server";
import { installationTeam } from "@/lib/github/store";

// The App's setup URL and its OAuth callback URL (both point here). GitHub
// sends ?installation_id=&setup_action=install|update|request, and with
// request_oauth_on_install also ?code= for the user who installed it.
//
// The installation_id is never trusted by itself: the code becomes a user
// token, and lib/github/link.ts decides from what GitHub says that token can
// see. Success seals the verified claim in a cookie and shows /github/link,
// where a team admin confirms. When GitHub sends no code (an update, or the
// OAuth step skipped), this starts GitHub's OAuth web flow itself, with a
// sealed state, and comes back here.

const OAUTH_COOKIE = "quod-gh-oauth";
const OAUTH_TTL = 600;
const ID = /^\d{1,15}$/;

export async function GET(request: NextRequest) {
  await connection();
  const here = requestOrigin(request.headers);
  const to = (path: string) => NextResponse.redirect(new URL(path, here), 303);
  const params = request.nextUrl.searchParams;
  const setupAction = params.get("setup_action");

  if (setupAction === "request") return to("/github/link?status=requested");

  const cfg = githubAppConfig();
  if (!cfg.ok) return to("/github/link?error=not-configured");
  const config = cfg.config;

  const viewer = await getViewer();
  const jar = await cookies();
  const code = params.get("code");
  let installationId = ID.test(params.get("installation_id") ?? "") ? Number(params.get("installation_id")) : null;

  // Back from the OAuth flow this route started: the installation id is the one sealed with the state.
  const state = params.get("state");
  if (state) {
    const sealed = jar.get(OAUTH_COOKIE)?.value;
    jar.delete(OAUTH_COOKIE);
    let stored: { purpose?: string; state?: string; installationId?: number } | null = null;
    try {
      stored = sealed ? await unsealData(sealed, { password: sealPassword(), ttl: OAUTH_TTL }) : null;
    } catch {
      stored = null;
    }
    if (!stored || stored.purpose !== "quod-gh-oauth" || stored.state !== state || typeof stored.installationId !== "number") return to("/github/link?error=state");
    installationId = stored.installationId;
  }

  if (!installationId) return to("/github/link?error=no-installation");

  if (!code) {
    // An update to an installation this team already has needs nothing more: webhooks carry repo changes.
    const teams = await getTeams();
    const team = currentTeam(viewer, teams);
    let linked: string | null = null;
    try {
      linked = await installationTeam(await getDb(), installationId);
    } catch {
      return to("/github/link?error=db");
    }
    if (team && linked === team.orgId) return to("/github/link?status=updated");
    // Ask GitHub who this is, with a state we can check.
    const s = randomBytes(24).toString("base64url");
    jar.set(OAUTH_COOKIE, await sealData({ purpose: "quod-gh-oauth", state: s, installationId }, { password: sealPassword(), ttl: OAUTH_TTL }), {
      httpOnly: true,
      secure: secureCookies(here),
      sameSite: "lax",
      path: "/github/installed",
      maxAge: OAUTH_TTL,
    });
    const authorize = new URL("https://github.com/login/oauth/authorize");
    authorize.searchParams.set("client_id", config.clientId);
    authorize.searchParams.set("redirect_uri", `${appOrigin(here)}/github/installed`);
    authorize.searchParams.set("state", s);
    return NextResponse.redirect(authorize, 303);
  }

  try {
    const [teams, db] = await Promise.all([getTeams(), getDb()]);
    const team = currentTeam(viewer, teams);
    if (!team && teams.length > 1) return to("/github/link?error=pick-team");
    const result = await verifyInstallationClaim({
      code,
      installationId,
      viewer,
      team,
      linkedTeamId: (id) => installationTeam(db, id),
      gh: githubForLinking(config),
    });
    if (!result.ok) {
      console.info("[quod-cloud] github installation not linked", { installationId, reason: result.reason });
      return to(`/github/link?error=${result.reason}`);
    }
    await saveClaim(result.claim, here);
    return to("/github/link");
  } catch (e) {
    console.warn("[quod-cloud] github installation check failed", { installationId, status: e instanceof GitHubApiError ? e.status : undefined, error: e instanceof Error ? e.name : "unknown" });
    return to("/github/link?error=github");
  }
}

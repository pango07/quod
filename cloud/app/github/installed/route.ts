import { connection, NextResponse, type NextRequest } from "next/server";
import { getViewer } from "@/lib/auth/session";
import { getTeams } from "@/lib/dal";
import { getDb } from "@/lib/db";
import { GitHubApiError } from "@/lib/github/api";
import { githubAppConfig } from "@/lib/github/env";
import { planInstalled } from "@/lib/github/flow-state";
import { verifyInstallationClaim } from "@/lib/github/link";
import { currentTeam, githubForLinking, routeOrigin, saveClaim, startFlowState, takeFlowState } from "@/lib/github/server";
import { installationTeam } from "@/lib/github/store";

// The App's setup URL and its OAuth callback URL (both point here). GitHub
// sends ?installation_id=&setup_action=install|update|request, our ?state=
// when the install started at /github/install, and with
// request_oauth_on_install a ?code= for the user who installed it.
//
// A code is consumed only with a state this browser got from Quod for the
// signed-in user (review M1). A code without one (someone else's
// install-return URL, or GitHub not echoing the state) is dropped, and the
// user goes through GitHub's OAuth with our own state instead. Then
// lib/github/link.ts decides from what the user's own token can see. Success
// seals the claim in a cookie and /github/link asks the admin to confirm.

export async function GET(request: NextRequest) {
  await connection();
  const origin = routeOrigin(request);
  const to = (path: string) => NextResponse.redirect(new URL(path, origin), 303);
  const params = request.nextUrl.searchParams;

  if (params.get("setup_action") === "request") return to("/github/link?status=requested");
  // The user declined GitHub's authorization (or it failed): stop, never loop back to authorize.
  if (params.get("error")) return to("/github/link?error=denied");
  const cfg = githubAppConfig();
  if (!cfg.ok) return to("/github/link?error=not-configured");
  const config = cfg.config;

  const viewer = await getViewer();
  const trusted = await takeFlowState(params.get("state"), viewer.id);
  const plan = planInstalled(params, trusted);

  const authorize = async (installationId: number) => {
    const state = await startFlowState(viewer.id, origin, installationId);
    const url = new URL("https://github.com/login/oauth/authorize");
    url.searchParams.set("client_id", config.clientId);
    url.searchParams.set("redirect_uri", `${origin}/github/installed`);
    url.searchParams.set("state", state);
    return NextResponse.redirect(url, 303);
  };

  switch (plan.kind) {
    case "requested":
      return to("/github/link?status=requested");
    case "no-installation":
      return to("/github/link?error=no-installation");
    case "authorize":
      if (plan.droppedCode) console.info("[quod-cloud] github code without our state dropped", { installationId: plan.installationId });
      return authorize(plan.installationId);
    case "check-linked": {
      // An update to an installation this team already has needs nothing more: webhooks carry repo changes.
      const team = currentTeam(viewer, await getTeams());
      let linked: string | null;
      try {
        linked = await installationTeam(await getDb(), plan.installationId);
      } catch {
        return to("/github/link?error=db");
      }
      if (team && linked === team.orgId) return to("/github/link?status=updated");
      return authorize(plan.installationId);
    }
    case "verify":
      break;
  }

  const { installationId, code } = plan;
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
    await saveClaim(result.claim, origin);
    return to("/github/link");
  } catch (e) {
    console.warn("[quod-cloud] github installation check failed", { installationId, status: e instanceof GitHubApiError ? e.status : undefined, error: e instanceof Error ? e.name : "unknown" });
    return to("/github/link?error=github");
  }
}

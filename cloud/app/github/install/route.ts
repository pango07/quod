import { connection, NextResponse, type NextRequest } from "next/server";
import { getViewer } from "@/lib/auth/session";
import { githubAppConfig } from "@/lib/github/env";
import { routeOrigin, startFlowState } from "@/lib/github/server";

// "Install on GitHub", started from Quod: GitHub's install page with a state
// sealed to this browser and user. GitHub hands the state back to the setup
// URL (/github/installed), which only consumes a code that carries it (M1).

export async function GET(request: NextRequest) {
  await connection();
  const origin = routeOrigin(request);
  const cfg = githubAppConfig();
  if (!cfg.ok) return NextResponse.redirect(new URL("/github/link?error=not-configured", origin), 303);
  const viewer = await getViewer();
  const state = await startFlowState(viewer.id, origin);
  const url = new URL(`https://github.com/apps/${encodeURIComponent(cfg.config.slug)}/installations/new`);
  url.searchParams.set("state", state);
  return NextResponse.redirect(url, 303);
}

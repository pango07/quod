import { authkit, handleAuthkitProxy } from "@workos-inc/authkit-nextjs";
import { NextResponse, type NextRequest } from "next/server";
import { resolveAuthMode } from "@/lib/auth/config";
import { sanitizeReturnTo } from "@/lib/auth/validate";
import { parseUi, UI_COOKIE } from "@/lib/ui-version";

// Next.js 16 proxy (formerly middleware). Every page needs a signed-in
// viewer except our sign-in pages and the OAuth callback. Signed-out
// visitors go to our own /sign-in, never to a hosted page.
const PUBLIC = ["/callback", "/sign-in", "/sign-up", "/verify-email", "/reset-password", "/invite", "/signed-out"];
const isPublic = (path: string) => PUBLIC.some((p) => path === p || path.startsWith(`${p}/`));

// Machine endpoints that authenticate themselves (the GitHub webhook checks its HMAC signature,
// the reconcile cron its CRON_SECRET): no session, no cookie.
const MACHINE = ["/api/github/webhook", "/api/github/reconcile"];

// A file at the root (/apple-touch-icon-precomposed.png, /robots.txt, ...) that no route serves:
// not a team page (team slugs have no dots), so 404 here, without reading a session.
// The real icons (app/favicon.ico, icon.png, apple-icon.png) never reach the proxy (see matcher).
const ROOT_FILE = /^\/[^/]*\.[A-Za-z0-9]{1,8}$/;
const ICONS = new Set(["/favicon.ico", "/icon.png", "/apple-icon.png"]);

/** ?ui=classic|plain on any page: remember the choice (lib/ui-version.ts). */
function rememberUi<R extends Response>(request: NextRequest, res: R): R {
  const ui = parseUi(request.nextUrl.searchParams.get("ui"));
  if (ui && res instanceof NextResponse) res.cookies.set(UI_COOKIE, ui, { path: "/", sameSite: "lax", httpOnly: true, secure: request.nextUrl.protocol === "https:", maxAge: 60 * 60 * 24 * 90 });
  return res;
}

export default async function proxy(request: NextRequest) {
  if (MACHINE.includes(request.nextUrl.pathname)) return NextResponse.next();
  if (ROOT_FILE.test(request.nextUrl.pathname) && !ICONS.has(request.nextUrl.pathname)) return new NextResponse("Not found", { status: 404, headers: { "cache-control": "no-store" } });
  const mode = resolveAuthMode(process.env);

  if (mode.kind === "misconfigured") {
    // Fail closed: no WorkOS config is never a way in.
    console.error(`[quod-cloud] refusing request: ${mode.problems.join(" ")}`);
    return new NextResponse("Quod Cloud is not configured. See the server log.", { status: 500, headers: { "cache-control": "no-store" } });
  }

  if (mode.kind === "demo") return rememberUi(request, NextResponse.next());

  // authkit() reads and refreshes the session cookie; we do not use its hosted sign-in URL.
  const { session, headers } = await authkit(request);
  const path = request.nextUrl.pathname;
  if (!session.user && !isPublic(path)) {
    // An EventSource (or its probe) cannot follow a sign-in redirect usefully: say 401 so the page can show "Signed out".
    if ((request.headers.get("accept") ?? "").includes("text/event-stream")) {
      return new NextResponse("Signed out", { status: 401, headers: { "cache-control": "no-store" } });
    }
    const returnTo = sanitizeReturnTo(path + request.nextUrl.search);
    const to = returnTo === "/" ? "/sign-in" : `/sign-in?returnTo=${encodeURIComponent(returnTo)}`;
    return handleAuthkitProxy(request, headers, { redirect: to });
  }
  return rememberUi(request, handleAuthkitProxy(request, headers));
}

export const config = {
  // Everything but Next.js build assets and the app icons (app/favicon.ico, icon.png, apple-icon.png).
  // A path like /acme/logo.png still goes through sign-in.
  matcher: ["/((?!_next/static|_next/image|favicon\\.ico$|icon\\.png$|apple-icon\\.png$).*)"],
};

import { authkit, handleAuthkitProxy } from "@workos-inc/authkit-nextjs";
import { NextResponse, type NextRequest } from "next/server";
import { resolveAuthMode } from "@/lib/auth/config";
import { sanitizeReturnTo } from "@/lib/auth/validate";

// Next.js 16 proxy (formerly middleware). Every page needs a signed-in
// viewer except our sign-in pages and the OAuth callback. Signed-out
// visitors go to our own /sign-in, never to a hosted page.
const PUBLIC = ["/callback", "/sign-in", "/sign-up", "/verify-email", "/reset-password", "/invite", "/signed-out"];
const isPublic = (path: string) => PUBLIC.some((p) => path === p || path.startsWith(`${p}/`));

export default async function proxy(request: NextRequest) {
  const mode = resolveAuthMode(process.env);

  if (mode.kind === "misconfigured") {
    // Fail closed: no WorkOS config is never a way in.
    console.error(`[quod-cloud] refusing request: ${mode.problems.join(" ")}`);
    return new NextResponse("Quod Cloud is not configured. See the server log.", { status: 500, headers: { "cache-control": "no-store" } });
  }

  if (mode.kind === "demo") return NextResponse.next();

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
  return handleAuthkitProxy(request, headers);
}

export const config = {
  // Everything but Next.js build assets. A path like /acme/logo.png still goes through sign-in.
  matcher: ["/((?!_next/static|_next/image|favicon\\.ico|icon\\.svg).*)"],
};

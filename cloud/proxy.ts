import { authkit, handleAuthkitProxy } from "@workos-inc/authkit-nextjs";
import { NextResponse, type NextRequest } from "next/server";
import { resolveAuthMode } from "@/lib/auth/config";

// Next.js 16 proxy (formerly middleware). Every page needs a signed-in
// viewer except the sign-in routes themselves.
const PUBLIC = new Set(["/callback", "/sign-in", "/signed-out"]);

export default async function proxy(request: NextRequest) {
  const mode = resolveAuthMode(process.env);

  if (mode.kind === "misconfigured") {
    // Fail closed: no WorkOS config is never a way in.
    console.error(`[quod-cloud] refusing request: ${mode.problems.join(" ")}`);
    return new NextResponse("Quod Cloud is not configured. See the server log.", { status: 500, headers: { "cache-control": "no-store" } });
  }

  if (mode.kind === "demo") return NextResponse.next();

  const { session, headers, authorizationUrl } = await authkit(request);
  if (!session.user && !PUBLIC.has(request.nextUrl.pathname) && authorizationUrl) {
    return handleAuthkitProxy(request, headers, { redirect: authorizationUrl });
  }
  return handleAuthkitProxy(request, headers);
}

export const config = {
  // Everything but Next.js build assets. A path like /acme/logo.png still goes through sign-in.
  matcher: ["/((?!_next/static|_next/image|favicon\\.ico|icon\\.svg).*)"],
};

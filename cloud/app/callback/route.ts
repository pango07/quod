import { getWorkOS, handleAuth } from "@workos-inc/authkit-nextjs";
import { connection, NextResponse, type NextRequest } from "next/server";
import { authMode } from "@/lib/auth/config";
import { clientId, fromError, finish, takeOAuthState } from "@/lib/auth/flow";

// The WorkOS redirect URI (NEXT_PUBLIC_WORKOS_REDIRECT_URI) points here.
// GitHub, Google and SSO sign-ins started on our own pages come back with
// our state; anything else (a flow AuthKit started) goes to AuthKit's handler.
const authkit = handleAuth({
  returnPathname: "/",
  // A stale or forged callback lands back on our sign-in page, not on a 500.
  onError: ({ request }) => NextResponse.redirect(new URL("/sign-in?error=provider", request.url)),
});

export async function GET(request: NextRequest) {
  await connection(); // per request, never prerendered
  if (authMode() === "demo") return NextResponse.redirect(new URL("/", request.url));

  const params = request.nextUrl.searchParams;
  const flow = await takeOAuthState(params.get("state"));
  if (!flow) return authkit(request);

  const back = (path: string) => NextResponse.redirect(new URL(path, request.url));
  const code = params.get("code");
  if (!code || params.get("error")) return back("/sign-in?error=provider");

  const ipAddress = (request.headers.get("x-forwarded-for")?.split(",")[0] ?? "").trim() || undefined;
  const userAgent = request.headers.get("user-agent")?.slice(0, 512) ?? undefined;
  try {
    const res = await getWorkOS().userManagement.authenticateWithCode({
      clientId: clientId(),
      code,
      codeVerifier: flow.verifier,
      invitationToken: flow.invitationToken,
      ipAddress,
      userAgent,
    });
    const done = await finish(res, flow.returnTo);
    return back(done.to);
  } catch (e) {
    const next = await fromError(e, { returnTo: flow.returnTo, invitationToken: flow.invitationToken });
    return next.kind === "error" ? back("/sign-in?error=provider") : NextResponse.redirect(new URL(next.to, request.url));
  }
}

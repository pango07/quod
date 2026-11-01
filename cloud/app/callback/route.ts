import { handleAuth } from "@workos-inc/authkit-nextjs";
import { connection, type NextRequest } from "next/server";
import { authMode } from "@/lib/auth/config";

// The WorkOS redirect URI (NEXT_PUBLIC_WORKOS_REDIRECT_URI) points here.
const workos = handleAuth({ returnPathname: "/" });

export async function GET(request: NextRequest) {
  await connection(); // per request, never prerendered
  if (authMode() === "demo") return Response.redirect(new URL("/", request.url));
  return workos(request);
}

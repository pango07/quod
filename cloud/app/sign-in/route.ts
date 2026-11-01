import { getSignInUrl } from "@workos-inc/authkit-nextjs";
import { redirect } from "next/navigation";
import { connection } from "next/server";
import { authMode } from "@/lib/auth/config";

// The WorkOS "Initiate login URI" (needed for impersonation and IdP-initiated sign-in).
export async function GET() {
  await connection(); // per request, never prerendered
  if (authMode() === "demo") redirect("/");
  redirect(await getSignInUrl());
}

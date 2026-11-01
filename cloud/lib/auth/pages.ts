import "server-only";
import { withAuth } from "@workos-inc/authkit-nextjs";
import { redirect } from "next/navigation";
import { authMode } from "./config";

/**
 * Previews of each sign-in state, for the local demo session only (authMode
 * "demo" already means QUOD_AUTH=demo and not production). Returns null
 * everywhere else, so a preview can never be forced on a real deploy.
 */
export function previewOf(value: string | string[] | undefined): string | null {
  if (authMode() !== "demo") return null;
  const v = Array.isArray(v0(value)) ? null : v0(value);
  return typeof v === "string" && /^[a-z-]{1,20}$/.test(v) ? v : null;
}
const v0 = (v: string | string[] | undefined) => (Array.isArray(v) ? v[0] : v);

export const isDemo = () => authMode() === "demo";

/** Someone already signed in who opens a sign-in page goes on to where they were headed. */
export async function redirectIfSignedIn(returnTo: string): Promise<void> {
  if (isDemo()) return;
  const { user } = await withAuth();
  if (user) redirect(returnTo);
}

export const DEMO_EMAIL = "jamie@example.com";

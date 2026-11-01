import { createHash, timingSafeEqual } from "node:crypto";

/**
 * Vercel Cron calls the route with `Authorization: Bearer $CRON_SECRET`.
 * Without CRON_SECRET set, nothing is allowed. Constant-time (hashes first,
 * so the lengths always match).
 */
export function cronAuthorized(header: string | null, secret: string | undefined): boolean {
  if (!secret || secret.length < 16 || !header) return false;
  const a = createHash("sha256").update(header).digest();
  const b = createHash("sha256").update(`Bearer ${secret}`).digest();
  return timingSafeEqual(a, b);
}

import { connection, NextResponse, type NextRequest } from "next/server";
import { getDb } from "@/lib/db";
import { cronAuthorized } from "@/lib/github/cron";
import { githubAppConfig } from "@/lib/github/env";
import { reconcile } from "@/lib/github/reconcile";
import { githubForReconcile } from "@/lib/github/server";

// Vercel Cron (vercel.json, every 15 minutes): redeliver failed GitHub
// webhooks and heal stuck runs and jobs (lib/github/reconcile.ts). Machine
// endpoint: the proxy lets it through; CRON_SECRET is the authentication.

export const maxDuration = 60;

export async function GET(request: NextRequest) {
  await connection();
  if (!cronAuthorized(request.headers.get("authorization"), process.env.CRON_SECRET)) {
    return NextResponse.json({ ok: false }, { status: 401, headers: { "cache-control": "no-store" } });
  }
  const cfg = githubAppConfig();
  if (!cfg.ok) return NextResponse.json({ ok: true, skipped: "github app not configured" }, { headers: { "cache-control": "no-store" } });
  try {
    const result = await reconcile({ db: getDb, gh: githubForReconcile(cfg.config), force: request.nextUrl.searchParams.get("force") === "1" });
    console.info("[quod-cloud] github reconcile", result);
    return NextResponse.json({ ok: true, ...result }, { headers: { "cache-control": "no-store" } });
  } catch (e) {
    console.error("[quod-cloud] github reconcile failed", { error: e instanceof Error ? e.name : "unknown" });
    return NextResponse.json({ ok: false }, { status: 500, headers: { "cache-control": "no-store" } });
  }
}

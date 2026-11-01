import { connection, NextResponse, type NextRequest } from "next/server";
import { getDb } from "@/lib/db";
import { cronAuthorized } from "@/lib/github/cron";
import { githubAppConfig } from "@/lib/github/env";
import { reconcileDaily } from "@/lib/github/reconcile";
import { githubForReconcile } from "@/lib/github/server";

// Vercel Cron, once a day (vercel.json; Hobby allows one daily cron): the
// full pass over every linked team (lib/github/reconcile.ts): redeliver the
// last 25 hours' failed webhooks, heal stuck runs and jobs, prune after 30
// days. Between runs, teams someone is watching are reconciled lazily by
// their live poller. Machine endpoint: the proxy lets it through;
// CRON_SECRET is the authentication.

export const maxDuration = 60;

export async function GET(request: NextRequest) {
  await connection();
  if (!cronAuthorized(request.headers.get("authorization"), process.env.CRON_SECRET)) {
    return NextResponse.json({ ok: false }, { status: 401, headers: { "cache-control": "no-store" } });
  }
  const cfg = githubAppConfig();
  if (!cfg.ok) return NextResponse.json({ ok: true, skipped: "github app not configured" }, { headers: { "cache-control": "no-store" } });
  try {
    const result = await reconcileDaily(await getDb(), githubForReconcile(cfg.config));
    console.info("[quod-cloud] github reconcile (daily)", result);
    return NextResponse.json({ ok: true, ...result }, { headers: { "cache-control": "no-store" } });
  } catch (e) {
    console.error("[quod-cloud] github reconcile failed", { error: e instanceof Error ? e.name : "unknown" });
    return NextResponse.json({ ok: false }, { status: 500, headers: { "cache-control": "no-store" } });
  }
}

import { after, connection, NextResponse, type NextRequest } from "next/server";
import { getDb } from "@/lib/db";
import { MAX_BODY, handleWebhook } from "@/lib/github/webhook";
import { pruneDeliveries } from "@/lib/github/store";

// GitHub App webhooks (hook_attributes.url in the manifest). Public: the
// proxy lets it through without a session; the X-Hub-Signature-256 HMAC is
// the authentication. Logs carry ids and event types only, never payloads.

let lastPrune = 0;

export async function POST(request: NextRequest) {
  await connection();
  const len = Number(request.headers.get("content-length") ?? "0");
  if (len > MAX_BODY) return reply(413, { ok: false, error: "too large" });
  const body = new Uint8Array(await request.arrayBuffer());

  let result;
  try {
    result = await handleWebhook({ db: await getDb(), secret: process.env.GITHUB_APP_WEBHOOK_SECRET?.trim(), headers: request.headers, body });
  } catch (e) {
    console.error("[quod-cloud] github webhook failed", {
      delivery: request.headers.get("x-github-delivery")?.slice(0, 100),
      event: request.headers.get("x-github-event")?.slice(0, 64),
      error: e instanceof Error ? e.name : "unknown",
    });
    // 5xx: GitHub marks the delivery failed, and a redelivery applies it (it was not recorded as seen).
    return reply(500, { ok: false, error: "not stored" });
  }

  if (result.status >= 400) {
    console.warn("[quod-cloud] github webhook refused", { status: result.status, delivery: result.delivery, event: result.event });
  } else {
    console.info("[quod-cloud] github webhook", { delivery: result.delivery, event: result.event, action: result.action, duplicate: result.body.duplicate ?? false, changes: result.versions?.length ?? 0 });
    // At most once a minute per instance, after the response so GitHub is answered first.
    if (Date.now() - lastPrune > 60_000) {
      lastPrune = Date.now();
      after(async () => {
        try {
          await pruneDeliveries(await getDb());
        } catch {
          lastPrune = 0; // the next delivery tries again
        }
      });
    }
  }
  return reply(result.status, result.body);
}

function reply(status: number, body: object) {
  return NextResponse.json(body, { status, headers: { "cache-control": "no-store" } });
}

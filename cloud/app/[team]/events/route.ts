import { connection, type NextRequest } from "next/server";
import { api } from "@/lib/api";
import { findTeam } from "@/lib/dal";
import { parseScope } from "@/lib/live/scope";
import { liveStream, SSE_HEADERS } from "@/lib/live/stream";

// GET /[team]/events?scope=team|repo|run|step: the dashboard's live stream (Server-Sent Events).
// Same session and membership check as the pages: a team you are not in is a 404.
// The stream closes after ~4 minutes, under the 300 s function limit, and the
// browser reconnects with Last-Event-ID; nothing is lost in between.
export const maxDuration = 300;

const NO_STORE = { "cache-control": "no-store" };
/** Event ids are opaque printable ASCII; anything else is refused before it reaches an adapter. */
const EVENT_ID = /^[\x21-\x7e]{1,128}$/;

export async function GET(request: NextRequest, { params }: RouteContext<"/[team]/events">) {
  await connection();
  const { team: slug } = await params;
  const ctx = await findTeam(slug);
  if (!ctx) return new Response("Not found", { status: 404, headers: NO_STORE });

  const scope = parseScope(request.nextUrl.searchParams);
  if (!scope) return new Response("Bad scope", { status: 400, headers: NO_STORE });

  const resumeFrom = request.headers.get("last-event-id") ?? request.nextUrl.searchParams.get("lastEventId");
  if (resumeFrom !== null && !EVENT_ID.test(resumeFrom)) return new Response("Bad Last-Event-ID", { status: 400, headers: NO_STORE });

  const cloud = api();
  const snapshot = await cloud.getActivity(ctx.team.slug, scope.kind === "team" ? {} : { repo: scope.repo });
  // Resume where the client left off; a new client starts exactly where its snapshot ends, so nothing in between is lost.
  const lastEventId = resumeFrom ?? snapshot.cursor;
  const stream = liveStream({
    snapshot,
    signal: request.signal,
    events: (signal) => cloud.subscribe(ctx.team.slug, scope, { lastEventId, signal }),
  });
  return new Response(stream, { headers: SSE_HEADERS });
}

/** The browser's probe after a failed stream: 200 (retry), 404 (no access), or 401 from the proxy (signed out). */
export async function HEAD(_request: NextRequest, { params }: RouteContext<"/[team]/events">) {
  await connection();
  const { team: slug } = await params;
  return new Response(null, { status: (await findTeam(slug)) ? 200 : 404, headers: NO_STORE });
}

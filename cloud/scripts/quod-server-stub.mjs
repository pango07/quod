#!/usr/bin/env node
// A stand-in for quod-server, for local development only: it answers with
// quod-server's recorded answers (lib/api/contract/quod-server, routes.json),
// as the real server does: the quod-protocol header, a bearer token, 404 for
// another team, 501 for what the server has not built, and the event stream
// (the recorded events, pings, then bye).
//
//   node scripts/quod-server-stub.mjs [--port 8787] [--down]
//
// --down answers 503 to everything, to see the "Quod data unavailable" states.
// Token: "acme-viewer-token" for team "acme" (as the recording used). Listens on
// 127.0.0.1 only and refuses to start with NODE_ENV=production.

import { readFileSync } from "node:fs";
import { createServer } from "node:http";
import { fileURLToPath } from "node:url";

if (process.env.NODE_ENV === "production") {
  console.error("quod-server-stub: development only (NODE_ENV=production)");
  process.exit(1);
}

const args = process.argv.slice(2);
const port = Number(args[args.indexOf("--port") + 1]) || 8787;
const down = args.includes("--down");
const DIR = fileURLToPath(new URL("../lib/api/contract/quod-server/", import.meta.url));
const read = (f) => readFileSync(DIR + f, "utf8");
const routes = JSON.parse(read("routes.json"));
const TOKEN = "acme-viewer-token";

const json = (res, status, body) => {
  res.writeHead(status, { "content-type": "application/json", "quod-protocol": "quod.v1" });
  res.end(typeof body === "string" ? body : JSON.stringify(body));
};
const error = (res, status, code, message) => json(res, status, { error: code, message });

const server = createServer((req, res) => {
  const url = new URL(req.url ?? "/", "http://localhost");
  const path = url.pathname + url.search;
  const method = req.method ?? "GET";
  console.log(`${method} ${path}`);
  if (url.pathname === "/healthz") return res.end("ok");
  if (down) return error(res, 503, "internal", "stub: --down");
  const events = url.pathname.endsWith("/events");
  if (req.headers["quod-protocol"] !== "quod.v1" && !(events && !req.headers["quod-protocol"]))
    return error(res, 400, "protocol", "this server speaks quod.v1; send `quod-protocol: quod.v1`");
  const auth = req.headers.authorization ?? "";
  if (!auth.startsWith("Bearer ")) return error(res, 401, "unauthorized", "no bearer token");
  if (auth.slice(7) !== TOKEN) return error(res, 401, "unauthorized", "unknown token");

  if (events) {
    if (url.pathname !== "/v1/teams/acme/events") return json(res, 404, read("team-404.json"));
    res.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-cache", "quod-protocol": "quod.v1" });
    // The recorded events after Last-Event-ID, then pings, then bye after a minute (the real server: 240 s).
    const after = Number(req.headers["last-event-id"] ?? "8");
    const blocks = read("events.sse")
      .split("\n\n")
      .filter((b) => b.startsWith("id: ") && Number(b.slice(4, b.indexOf("\n"))) > after);
    for (const b of blocks) res.write(`${b}\n\n`);
    const ping = setInterval(() => res.write(": ping\n\n"), 15_000);
    const bye = setTimeout(() => res.end("event: bye\ndata: {}\n\n"), 60_000);
    req.on("close", () => (clearInterval(ping), clearTimeout(bye)));
    return;
  }

  const r = routes[`${method} ${path}`];
  if (r) return json(res, r[0], read(r[1]));
  if (/^\/v1\/teams\/acme\/repos\/[^/]+\/runs\/\d+$/.test(url.pathname)) return error(res, 404, "not_found", "no such run");
  if (url.pathname.startsWith("/v1/teams/acme/")) return error(res, 404, "not_found", "no such repo");
  return json(res, 404, read("team-404.json"));
});

server.listen(port, "127.0.0.1", () => console.log(`quod-server-stub on http://localhost:${port}${down ? " (down: 503)" : ""}`));

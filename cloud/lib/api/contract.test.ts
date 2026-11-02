import { describe, expect, it } from "vitest";
import { answer, fixture, KEY_TEST, KEY_VET } from "./contract/server";
import { HttpCloudApi, parseLiveEvent, type ServerCredentials } from "./http";
import { ApiForbiddenError, ApiInvalidRequestError, ApiNotAvailableError, ApiUnavailableError, type LiveEvent } from "./types";

// The contract: quod-server's own answers (recorded from crates/quod-server,
// contract/quod-server/*) through the dashboard's HTTP adapter. If the server
// changes a field name, an enum or an error shape, these fail.

const STATIC: ServerCredentials = { team: "acme", token: "acme-viewer-token", kind: "static" };

interface Seen {
  method: string;
  path: string;
  headers: Record<string, string>;
  init: RequestInit;
}

/** A fetch that answers like the recorded server, and remembers each request. */
function recordedServer(override?: (method: string, path: string, n: number) => Response | Promise<Response> | null) {
  const seen: Seen[] = [];
  const f = (async (input: URL | RequestInfo, init: RequestInit = {}) => {
    const url = new URL(String(input));
    const method = init.method ?? "GET";
    const path = url.pathname + url.search;
    seen.push({ method, path, headers: { ...(init.headers as Record<string, string>) }, init });
    const o = await override?.(method, path, seen.length);
    if (o) return o;
    const a = answer(method, path);
    return new Response(a.body, { status: a.status, headers: { "content-type": a.type, "quod-protocol": "quod.v1" } });
  }) as typeof fetch;
  return { f, seen };
}

const adapter = (f: typeof fetch, creds: ServerCredentials | null = STATIC, extra: Partial<ConstructorParameters<typeof HttpCloudApi>[0]> = {}) =>
  new HttpCloudApi({ baseUrl: "https://quod.example.com", credentials: async () => creds, fetch: f, sleep: () => new Promise((r) => setTimeout(r, 0)), ...extra });

describe("quod.v1 contract: quod-server's answers through the HTTP adapter", () => {
  it("reads every dashboard endpoint the server serves", async () => {
    const { f } = recordedServer();
    const api = adapter(f);

    expect(await api.getTeamHealth("acme")).toEqual({ wrongReplays: 0, spotChecks: 0, pausedSteps: 0, repos: 1 });

    const ov = await api.getOverview("acme", "7d");
    expect(ov.totals).toMatchObject({ range: "7d", stepsReplayed: 2, stepsTotal: 6, savedMinutes: 1, usdPerMinute: 0.008, wrongReplays: 0 });
    expect(ov.repos).toEqual([expect.objectContaining({ slug: "app", fullName: "acme/app", hasQuodData: true, lastRunStatus: "passed" })]);
    expect(ov.trust).toEqual({ proven: 2, proving: 0, watching: 0, spotChecksPassed: 0, paused: [] });
    // quod-server sends only days with steps; the chart gets every day of the range, the recorded one last.
    expect(ov.days).toHaveLength(7);
    expect(ov.days.slice(0, 6).every((d) => d.replayedMinutes === 0 && d.ranMinutes === 0)).toBe(true);
    expect(ov.days[6]).toEqual({ day: ov.asOf.slice(0, 10), replayedMinutes: expect.any(Number), ranMinutes: expect.any(Number) });
    expect(ov.days[6].replayedMinutes).toBeGreaterThan(0);
    expect(ov.quod).toEqual({ state: "connected" });

    expect((await api.listRepos("acme")).map((r) => r.slug)).toEqual(["app"]);

    const repo = await api.getRepo("acme", "app");
    expect(repo?.summary.fullName).toBe("acme/app");
    expect(repo?.latestRun?.number).toBe(3);
    expect(repo?.runTimes.map((p) => p.number)).toEqual([1, 2, 3]);
    expect(repo?.steps.map((s) => [s.command, s.trust.kind])).toEqual([
      ["go vet ./...", "replaying"],
      ["go test ./...", "replaying"],
    ]);

    const runs = await api.listRuns("acme", { limit: 10 });
    expect(runs.runs.map((r) => [r.number, r.stepsReplayed, r.stepsTotal, r.status, r.event])).toEqual([
      [3, 2, 2, "passed", "push"],
      [2, 0, 2, "passed", "push"],
      [1, 0, 2, "passed", "push"],
    ]);

    const run3 = await api.getRun("acme", "app", 3);
    expect(run3?.jobs).toHaveLength(1);
    expect(run3?.jobs[0]).toMatchObject({ id: "test", name: "test" });
    expect(run3?.jobs[0].steps.map((s) => [s.name, s.outcome, s.why.kind])).toEqual([
      ["Test", "replayed", "replayed"],
      ["Vet", "replayed", "replayed"],
    ]);
    expect(run3?.jobs[0].steps[0].why).toMatchObject({ kind: "replayed", stepKey: KEY_TEST, sourceRun: 2, proof: { have: 2, need: 2 } });
    const run1 = await api.getRun("acme", "app", 1);
    expect(run1?.jobs[0].steps[0].why).toMatchObject({ kind: "first-run", proof: { have: 1, need: 2 }, recordedInputs: 10 });

    const steps = await api.listSteps("acme");
    expect(steps.map((s) => s.key)).toEqual([KEY_VET, KEY_TEST]);
    const st = await api.getStepTrust("acme", "app", KEY_VET);
    expect(st).toMatchObject({ key: KEY_VET, proofsNeeded: 2, divergence: null, inputs: { filesRead: 5, foldersListed: 4, pathsNotFound: 1, networkAccess: 0 } });
    expect(st?.history.map((h) => h.kind)).toEqual(["replayed", "consistent", "first-recorded"]);

    const idx = await api.getSearchIndex("acme");
    expect(idx.repos).toEqual([{ slug: "app", fullName: "acme/app" }]);
    expect(idx.steps.map((s) => s.trust)).toEqual(["replaying", "replaying"]);

    const act = await api.getActivity("acme");
    expect(act).toMatchObject({ jobs: [], cursor: "8", simulated: false });
  });

  it("sends quod.v1's protocol header and the bearer token, and nothing else of the viewer's (no cookie)", async () => {
    const { f, seen } = recordedServer();
    await adapter(f).getOverview("acme", "7d");
    expect(seen).toHaveLength(1);
    const h = seen[0].headers;
    expect(h["quod-protocol"]).toBe("quod.v1");
    expect(h.authorization).toBe("Bearer acme-viewer-token");
    expect(Object.keys(h).map((k) => k.toLowerCase()).sort()).toEqual(["accept", "authorization", "quod-protocol"]);
    expect(seen[0].init).toMatchObject({ credentials: "omit", redirect: "error", cache: "no-store" });
  });

  it("uses quod-server's team id, never the dashboard's slug, in the path", async () => {
    const { f, seen } = recordedServer();
    const api = new HttpCloudApi({ baseUrl: "https://quod.example.com", credentials: async (t) => (t === "acme-inc" ? STATIC : null), fetch: f });
    await api.getTeamHealth("acme-inc");
    expect(seen[0].path).toBe("/v1/teams/acme/health");
  });

  it("501: 'not available yet' with the server's reason, for each endpoint it has not built", async () => {
    const api = adapter(recordedServer().f);
    for (const call of [
      () => api.listRunners("acme"),
      () => api.getConnectSetup("acme"),
      () => api.saveConnectChoice("acme", { store: "cloud", host: "github" }),
      () => api.requestRerunWithoutQuod("acme", "app", 3),
      () => api.setProofsNeeded("acme", "app", KEY_VET, 3),
    ]) {
      const e = await call().catch((x: unknown) => x);
      expect(e).toBeInstanceOf(ApiNotAvailableError);
      expect((e as ApiNotAvailableError).detail.length).toBeGreaterThan(10);
    }
    const detail = await api.requestRerunWithoutQuod("acme", "app", 3).catch((e: ApiNotAvailableError) => e.detail);
    expect(detail).toBe(JSON.parse(fixture("rerun-501.json")).message);
  });

  it("404: a missing repo, run or step is null; a team or repo quod-server does not have yet has no Quod data", async () => {
    const api = adapter(recordedServer().f);
    expect(await api.getRepo("acme", "nope")).toBeNull();
    expect(await api.getRun("acme", "app", 99)).toBeNull();
    expect(await api.getStepTrust("acme", "app", "0".repeat(64))).toBeNull();
    expect((await api.listRuns("acme", { repo: "nope" })).runs).toEqual([]);
    const other = adapter(recordedServer().f, { ...STATIC, team: "beta" });
    expect((await other.getOverview("beta", "7d")).repos).toEqual([]);
    expect(await other.listRepos("beta")).toEqual([]);
    expect(await other.getTeamHealth("beta")).toEqual({ wrongReplays: 0, spotChecks: 0, pausedSteps: 0, repos: 0 });
  });

  it("no credentials for a team: quod-server is not asked, and Quod's side is empty", async () => {
    const { f, seen } = recordedServer();
    const api = adapter(f, null);
    expect((await api.getOverview("acme", "7d")).quod).toEqual({ state: "off" });
    expect(await api.getRun("acme", "app", 3)).toBeNull();
    expect(seen).toEqual([]);
  });

  it("401 and 403: sign in again only when the viewer's own token expired; a refused deploy secret is a misconfiguration", async () => {
    const refusing = recordedServer(() => new Response(fixture("unauthorized-401.json"), { status: 401 })).f;
    const staticErr = await adapter(refusing).getOverview("acme", "7d").catch((e: unknown) => e);
    expect(staticErr).toBeInstanceOf(ApiUnavailableError);
    expect((staticErr as ApiUnavailableError).status).toBe(401);

    const jwt = (exp: number) => `x.${Buffer.from(JSON.stringify({ exp })).toString("base64url")}.y`;
    const now = Date.now();
    const expired = await adapter(refusing, { team: "org_1", token: jwt(now / 1000 - 10), kind: "viewer" })
      .getOverview("acme", "7d")
      .catch((e: unknown) => e);
    expect(expired).toBeInstanceOf(ApiForbiddenError);
    expect((expired as ApiForbiddenError).status).toBe(401);
    // A fresh viewer token refused: the server's viewer auth is wrong, not the session. Never a sign-in loop.
    const fresh = await adapter(refusing, { team: "org_1", token: jwt(now / 1000 + 300), kind: "viewer" })
      .getOverview("acme", "7d")
      .catch((e: unknown) => e);
    expect(fresh).toBeInstanceOf(ApiUnavailableError);

    const forbidden = recordedServer(() => new Response(fixture("runner-403.json"), { status: 403 })).f;
    const e403 = await adapter(forbidden).getRepo("acme", "app").catch((e: unknown) => e);
    expect(e403).toBeInstanceOf(ApiForbiddenError);
    expect((e403 as ApiForbiddenError).status).toBe(403);
  });

  it("a protocol mismatch, a 5xx, a timeout or a body that is not quod.v1 is 'unavailable'", async () => {
    const proto = recordedServer(() => new Response(fixture("protocol-400.json"), { status: 400 })).f;
    const e = await adapter(proto).getOverview("acme", "7d").catch((x: unknown) => x);
    expect(e).toMatchObject({ name: "ApiUnavailableError", status: 400, code: "protocol" });

    const bad = recordedServer(() => new Response(JSON.stringify({ asOf: "now", totals: {} }), { status: 200 })).f;
    await expect(adapter(bad).getOverview("acme", "7d")).rejects.toThrow(/not quod\.v1/);

    const html = recordedServer(() => new Response("<html>", { status: 200 })).f;
    await expect(adapter(html).getTeamHealth("acme")).rejects.toBeInstanceOf(ApiUnavailableError);

    const slow = (async (_u: unknown, init: RequestInit) =>
      new Promise((_ok, fail) => init.signal?.addEventListener("abort", () => fail(Object.assign(new Error("timeout"), { name: "TimeoutError" }))))) as typeof fetch;
    const t = await adapter(slow, STATIC, { timeoutMs: 20, retries: 0 }).getTeamHealth("acme").catch((x: unknown) => x);
    expect(t).toBeInstanceOf(ApiUnavailableError);
    expect((t as Error).message).toMatch(/did not answer/);
  });

  it("retries GETs (and only GETs) once on 502/503/504 and network errors, never after a timeout", async () => {
    let n = 0;
    const flaky = recordedServer((m) => (m === "GET" && ++n <= 1 ? new Response("{}", { status: 503 }) : null));
    expect((await adapter(flaky.f).getTeamHealth("acme")).repos).toBe(1);
    expect(flaky.seen).toHaveLength(2);

    const down = recordedServer(() => new Response('{"error":"internal","message":"internal error"}', { status: 502 }));
    await expect(adapter(down.f).getTeamHealth("acme")).rejects.toBeInstanceOf(ApiUnavailableError);
    expect(down.seen).toHaveLength(2);

    const post = recordedServer(() => new Response("{}", { status: 503 }));
    await expect(adapter(post.f).requestRerunWithoutQuod("acme", "app", 3)).rejects.toBeInstanceOf(ApiUnavailableError);
    expect(post.seen).toHaveLength(1);

    const internal = recordedServer(() => new Response('{"error":"internal","message":"internal error"}', { status: 500 }));
    await expect(adapter(internal.f).getTeamHealth("acme")).rejects.toMatchObject({ status: 500, code: "internal" });
    expect(internal.seen).toHaveLength(1);
  });

  it("refuses ids that could walk the server's paths before any request", async () => {
    const { f, seen } = recordedServer();
    const api = adapter(f);
    await expect(api.getRepo("acme", "../x")).rejects.toBeInstanceOf(ApiInvalidRequestError);
    await expect(api.getRun("acme", "app", 0)).rejects.toBeInstanceOf(ApiInvalidRequestError);
    await expect(api.getStepTrust("acme", "app", "b3:abc")).rejects.toBeInstanceOf(ApiInvalidRequestError);
    expect(seen).toEqual([]);
  });
});

describe("quod.v1 contract: the event stream", () => {
  /** The recorded stream, once; then an empty one that only says bye. */
  function streamServer() {
    const seen: { lastEventId: string | null; path: string }[] = [];
    const f = (async (input: URL | RequestInfo, init: RequestInit = {}) => {
      const url = new URL(String(input));
      const h = init.headers as Record<string, string>;
      seen.push({ lastEventId: h["last-event-id"] ?? null, path: url.pathname + url.search });
      const body = seen.length === 1 ? fixture("events.sse") : ": ping\n\nevent: bye\ndata: {}\n\n";
      return new Response(body, { status: 200, headers: { "content-type": "text/event-stream" } });
    }) as typeof fetch;
    return { f, seen };
  }

  it("yields quod-server's step.finished and trust.changed as LiveEvents, and resumes with Last-Event-ID after bye", async () => {
    const { f, seen } = streamServer();
    const ac = new AbortController();
    const got: LiveEvent[] = [];
    for await (const e of adapter(f).subscribe("acme", { kind: "team" }, { lastEventId: "0", signal: ac.signal })) {
      got.push(e);
      if (seen.length >= 2 && got.length === 8) ac.abort();
      if (got.length >= 8) ac.abort();
    }
    expect(got.map((e) => [e.id, e.type])).toEqual([
      ["1", "step.finished"],
      ["2", "step.finished"],
      ["3", "step.finished"],
      ["4", "trust.changed"],
      ["5", "step.finished"],
      ["6", "trust.changed"],
      ["7", "step.finished"],
      ["8", "step.finished"],
    ]);
    const last = got[7];
    expect(last).toMatchObject({ type: "step.finished", repoSlug: "app", run: 3, jobId: "test", index: 4, stepKey: KEY_VET, step: { name: "Vet", outcome: "replayed" } });
    expect(got[3]).toMatchObject({ type: "trust.changed", key: KEY_TEST, trust: { kind: "replaying" }, event: { kind: "proven" } });
    expect(seen[0]).toEqual({ lastEventId: "0", path: "/v1/teams/acme/events?scope=team" });
  });

  it("reconnects after bye from the last id it saw, and keeps the scope", async () => {
    const { f, seen } = streamServer();
    const ac = new AbortController();
    const it = adapter(f).subscribe("acme", { kind: "run", repo: "app", run: 3 }, { lastEventId: null, signal: ac.signal })[Symbol.asyncIterator]();
    for (let i = 0; i < 8; i++) await it.next();
    // The ninth read needs a new connection: the first said bye after event 8.
    const p = it.next();
    await new Promise((r) => setTimeout(r, 20));
    ac.abort();
    await p;
    expect(seen[0]).toEqual({ lastEventId: null, path: "/v1/teams/acme/events?scope=run&repo=app&run=3" });
    expect(seen[1]).toEqual({ lastEventId: "8", path: "/v1/teams/acme/events?scope=run&repo=app&run=3" });
  });

  it("stops on a refused viewer, and holds quiet for a team the server does not have", async () => {
    const refused = (async () => new Response("{}", { status: 403 })) as unknown as typeof fetch;
    const it = adapter(refused).subscribe("acme", { kind: "team" }, { signal: new AbortController().signal })[Symbol.asyncIterator]();
    await expect(it.next()).rejects.toBeInstanceOf(ApiForbiddenError);

    let calls = 0;
    const missing = (async () => (calls++, new Response(fixture("team-404.json"), { status: 404 }))) as unknown as typeof fetch;
    const ac = new AbortController();
    const quiet = adapter(missing, STATIC, { sleep: (ms, s) => new Promise((r) => (s?.addEventListener("abort", () => r()), setTimeout(r, Math.min(ms, 5)))) });
    const run = (async () => {
      const out: LiveEvent[] = [];
      for await (const e of quiet.subscribe("acme", { kind: "team" }, { signal: ac.signal })) out.push(e);
      return out;
    })();
    setTimeout(() => ac.abort(), 30);
    expect(await run).toEqual([]);
    expect(calls).toBeGreaterThan(0);
  });

  it("parseLiveEvent: quod-server's empty step key is null; unknown types and broken data are dropped", () => {
    const data = JSON.stringify({ at: "2026-11-02T00:00:00Z", repoSlug: "app", run: 1, jobId: "test", index: 0, stepKey: "", step: { name: "x", outcome: "ran", seconds: 1, why: { kind: "network" } } });
    expect(parseLiveEvent("step.finished", data, "9")).toMatchObject({ id: "9", type: "step.finished", stepKey: null });
    expect(parseLiveEvent("bye", "{}", undefined)).toBeNull();
    expect(parseLiveEvent("step.finished", "{", "1")).toBeNull();
    expect(parseLiveEvent("step.finished", JSON.stringify({ at: "x" }), "1")).toBeNull();
    expect(parseLiveEvent("made.up", data, "1")).toBeNull();
  });
});

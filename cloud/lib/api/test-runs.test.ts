import { describe, expect, it } from "vitest";
import { FixtureCloudApi } from "./fixture";
import { HttpCloudApi, parseLiveEvent, type ServerCredentials } from "./http";
import { mapEvent, buildRepoMap, mapTestRun, mapTestSuite } from "./repo-map";
import { ApiUnavailableError, type LiveEvent } from "./types";
import { sampleTestRuns, sampleTestSuites, summaryOnly } from "./fixture-tests";
import { testRunHeadline } from "@/lib/test-runs";

// Test runs (test-level replay) through the adapters: an older quod-server
// without the endpoints (404 or 501) must leave the dashboard quietly
// without test sections, never an error.

const CREDS: ServerCredentials = { team: "acme", token: "acme-viewer-token", kind: "static" };

function server(answer: (path: string) => Response) {
  const seen: string[] = [];
  const f = (async (input: URL | RequestInfo) => {
    const url = new URL(String(input));
    seen.push(url.pathname + url.search);
    return answer(url.pathname + url.search);
  }) as typeof fetch;
  const api = new HttpCloudApi({ baseUrl: "https://quod.example.com", credentials: async () => CREDS, fetch: f, sleep: async () => undefined });
  return { api, seen };
}

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
const sample = sampleTestRuns(Date.parse("2026-11-02T12:00:00Z"));
const critical = sample.find((r) => r.id === "tr_4182c")!;

describe("test runs over HTTP", () => {
  it("an older server (404 on every test endpoint) gives null: the UI hides test sections", async () => {
    const { api } = server(() => json({ error: "not_found", message: "no such route" }, 404));
    expect(await api.listTestRuns("acme")).toBeNull();
    expect(await api.getTestRun("acme", "tr_1")).toBeNull();
    expect(await api.getTestStats("acme")).toBeNull();
  });

  it("501 (not built on this server) is null too, not an error", async () => {
    const { api } = server(() => json({ error: "not_implemented", message: "later" }, 501));
    expect(await api.listTestRuns("acme", { repo: "app" })).toBeNull();
    expect(await api.getTestRun("acme", "tr_1")).toBeNull();
    expect(await api.getTestStats("acme")).toBeNull();
  });

  it("a server that is down is still 'unavailable' (the page shows its section card)", async () => {
    const { api } = server(() => json({ error: "internal", message: "boom" }, 500));
    await expect(api.listTestRuns("acme")).rejects.toBeInstanceOf(ApiUnavailableError);
  });

  it("reads runs, one run and the stats; sends repo and a clamped limit", async () => {
    const stats = { asOf: "2026-11-02T12:00:00Z", today: { savedMs: 79240, runs: 1 }, week: { savedMs: 300000, runs: 4, filesTotal: 700, filesSkipped: 640, skippedPct: 91.4 }, wrongResults: 0, audits: { checked: 4, passed: 4 }, recent: [summaryOnly(critical)] };
    const { api, seen } = server((p) => (p.startsWith("/v1/teams/acme/test-runs?") ? json({ runs: [summaryOnly(critical)] }) : p.startsWith("/v1/teams/acme/test-runs/") ? json(critical) : json(stats)));
    expect((await api.listTestRuns("acme", { repo: "web", limit: 5000 }))?.map((r) => r.id)).toEqual(["tr_4182c"]);
    expect(seen[0]).toBe("/v1/teams/acme/test-runs?repo=web&limit=200");
    const run = await api.getTestRun("acme", "tr_4182c");
    expect(run?.ranFiles).toHaveLength(13);
    expect(run?.shadow?.kind).toBe("shadow");
    expect((await api.getTestStats("acme"))?.recent).toHaveLength(1);
  });

  it("a body that is not a test run is 'unavailable', never a crash in a component", async () => {
    const { api } = server(() => json({ runs: [{ id: 1 }] }));
    await expect(api.listTestRuns("acme")).rejects.toBeInstanceOf(ApiUnavailableError);
  });
});

describe("test.run events", () => {
  it("are parsed from the stream, and broken ones are skipped", () => {
    const run = summaryOnly(critical);
    const e = parseLiveEvent("test.run", JSON.stringify({ id: "7", type: "test.run", at: run.at, repoSlug: "web", run }), "7");
    expect(e).toMatchObject({ type: "test.run", repoSlug: "web", run: { id: "tr_4182c" } });
    expect(parseLiveEvent("test.run", JSON.stringify({ id: "8", at: run.at, repoSlug: "web", run: { id: "x" } }), "8")).toBeNull();
  });

  it("carry the dashboard's repo slug, in the event and in its run", () => {
    const map = buildRepoMap([{ slug: "web-app", fullName: "quodci/web.app" }], [{ slug: "web.app", fullName: "quodci/web.app" }]);
    const e = { id: "1", at: critical.at, type: "test.run", repoSlug: "web-app", run: { ...summaryOnly(critical), repoSlug: "web-app" } } as LiveEvent;
    const m = mapEvent(map, e);
    expect(m.type === "test.run" && [m.repoSlug, m.run.repoSlug]).toEqual(["web.app", "web.app"]);
  });
});

describe("test runs in the sample data", () => {
  const api = new FixtureCloudApi(undefined, 0);

  it("has the headline run", async () => {
    const run = await api.getTestRun("quodci", "tr_4182c");
    expect(run && testRunHeadline(run)).toBe("vitest · packages/backend · 184 test files: 171 skipped (unchanged), 13 ran, saved ≈ 79 s");
    expect(await api.getTestRun("quodci", "nope")).toBeNull();
  });

  it("adds up, and real teams have none (hidden)", async () => {
    const stats = await api.getTestStats("quodci");
    expect(stats?.wrongResults).toBe(0);
    expect(stats?.audits.passed).toBe(stats?.audits.checked);
    expect(stats?.recent.length).toBeGreaterThan(0);
    expect((await api.listTestRuns("quodci", { repo: "web", limit: 3 }))?.length).toBe(3);
    const real = new FixtureCloudApi(undefined, 0, async () => "org_real");
    expect(await real.getTestStats("quodci")).toBeNull();
    expect(await real.listTestRuns("quodci")).toBeNull();
    expect(await real.listTestSuites("quodci")).toBeNull();
  });

  it("has test suites for Trust", async () => {
    const suites = await api.listTestSuites("quodci");
    expect(suites?.map((s) => [s.repoSlug, s.suite, s.totals.files])).toEqual([
      ["web", "packages/backend", 184],
      ["web", "packages/web", 96],
      ["web", "packages/contract", 7],
      ["quod", ".", 9],
    ]);
  });
});

describe("test suites (Trust) over HTTP", () => {
  const suite = sampleTestSuites(Date.parse("2026-11-02T12:00:00Z"))[0];

  it("reads GET /test-suites", async () => {
    const { api, seen } = server(() => json({ suites: [suite] }));
    const got = await api.listTestSuites("acme");
    expect(seen).toEqual(["/v1/teams/acme/test-suites"]);
    expect(got?.[0].files).toHaveLength(184);
    expect(got?.[0].totals.paused).toBe(1);
  });

  it("an older server (404) or one without it (501) gives null, not an error", async () => {
    expect(await server(() => json({ error: "not_found", message: "no such route" }, 404)).api.listTestSuites("acme")).toBeNull();
    expect(await server(() => json({ error: "not_implemented", message: "later" }, 501)).api.listTestSuites("acme")).toBeNull();
  });

  it("down, or a body that is not suites: 'unavailable' (the page's section card)", async () => {
    await expect(server(() => json({ error: "internal", message: "boom" }, 500)).api.listTestSuites("acme")).rejects.toBeInstanceOf(ApiUnavailableError);
    await expect(server(() => json({ suites: [{ repoSlug: "web" }] })).api.listTestSuites("acme")).rejects.toBeInstanceOf(ApiUnavailableError);
  });
});

describe("test runs and suites of a repo quod-server's repo list does not have", () => {
  // A GitHub repo that only runs `quod vitest` (pango07/ci-bench): quod-server
  // knows it from its test runs only, maybe under another slug.
  const map = buildRepoMap([{ slug: "app", fullName: "acme/app" }], [
    { slug: "App", fullName: "acme/App" },
    { slug: "ci-bench", fullName: "pango07/ci-bench" },
  ]);

  it("lands on the GitHub slug by full name", () => {
    const run = { ...summaryOnly(critical), repoSlug: "pango07-ci-bench", repository: "pango07/ci-bench" };
    expect(mapTestRun(map, run).repoSlug).toBe("ci-bench");
    expect(mapTestSuite(map, { ...sampleTestSuites()[0], repoSlug: "pango07-ci-bench", repository: "pango07/ci-bench" }).repoSlug).toBe("ci-bench");
    // Quod's own repos still map as before.
    expect(mapTestRun(map, { ...summaryOnly(critical), repoSlug: "app", repository: "acme/app" }).repoSlug).toBe("App");
  });
});

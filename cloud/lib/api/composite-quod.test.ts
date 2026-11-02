import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { migratedDb, truncateAll } from "@/lib/db/testing";
import type { Db } from "@/lib/db/types";
import { readCached } from "@/lib/github/protection";
import { linkInstallation } from "@/lib/github/store";
import { CompositeCloudApi } from "./composite";
import { answer } from "./contract/server";
import { quodMarks } from "./github-merge";
import { HttpCloudApi } from "./http";
import { NoQuodCloudApi } from "./none";
import { buildRepoMap, mapEvent } from "./repo-map";
import type { CloudApi, LiveEvent } from "./types";

// GitHub's data and quod-server's, merged per repository: the recorded
// server (contract/quod-server) next to a linked installation in PGlite.

vi.mock("server-only", () => ({}));

let db: Db;
const ORG = "org_acme";
const OWNER = { id: 500, login: "acme", type: "Organization" };
// GitHub spells it "App"; quod-server's slug is "app" (it lowercases).
const APP = { id: 7, name: "App", full_name: "acme/App", private: true, owner: OWNER };
const WEB = { id: 8, name: "web", full_name: "acme/web", private: true, owner: OWNER };

function recorded(down = false) {
  const seen: string[] = [];
  const f = (async (input: URL | RequestInfo, init: RequestInit = {}) => {
    const url = new URL(String(input));
    seen.push(`${init.method ?? "GET"} ${url.pathname}${url.search}`);
    if (down) throw new TypeError("fetch failed");
    const a = answer(init.method ?? "GET", url.pathname + url.search);
    return new Response(a.body, { status: a.status });
  }) as typeof fetch;
  return { f, seen };
}

function composite(inner: CloudApi, opts: { protection?: boolean } = {}) {
  return new CompositeCloudApi(inner, {
    github: {
      configured: () => true,
      appReady: () => true,
      db: async () => db,
      ...(opts.protection === false ? {} : { protection: async (teamId, d, repos) => (await readCached(d, teamId, repos.map((r) => r.id))).byRepo }),
    },
    orgOf: async () => ORG,
    onQuodError: () => {},
  });
}

const http = (f: typeof fetch) => new HttpCloudApi({ baseUrl: "https://quod.example.com", credentials: async () => ({ team: "acme", token: "acme-viewer-token", kind: "static" }), fetch: f, retries: 0 });

beforeAll(async () => {
  db = await migratedDb();
});
afterAll(async () => db.close());
beforeEach(async () => {
  await truncateAll(db);
  await db.tx((q) => linkInstallation(q, { installation: { id: 77, account: OWNER }, teamId: ORG, userId: "user_1", githubUserId: 1, repos: [APP, WEB] }));
});

describe("composite: GitHub and quod-server, one row per repository", () => {
  it("a repo Quod has data for shows Quod's numbers under GitHub's slug; one it has not, GitHub's", async () => {
    const cloud = composite(http(recorded().f));
    const ov = await cloud.getOverview("acme", "7d");
    expect(ov.quod).toEqual({ state: "connected" });
    expect(ov.repos.map((r) => [r.slug, r.fullName, r.hasQuodData])).toEqual([
      ["App", "acme/app", true],
      ["web", "acme/web", false],
    ]);
    expect(ov.repos[0]).toMatchObject({ replayRate: expect.any(Number), savedMinutes: 1, github: { latestRun: null }, protection: { state: "unknown" } });
    expect(ov.totals.stepsReplayed).toBe(2);
  });

  it("maps the slug both ways: pages ask for App, quod-server is asked for app, answers come back as App", async () => {
    const server = recorded();
    const cloud = composite(http(server.f));
    const repo = await cloud.getRepo("acme", "App");
    expect(repo?.summary).toMatchObject({ slug: "App", hasQuodData: true });
    expect(repo?.latestRun?.repoSlug).toBe("App");
    expect(repo?.steps.every((s) => s.repoSlug === "App")).toBe(true);
    expect((await cloud.getRun("acme", "App", 3))?.repoSlug).toBe("App");
    expect((await cloud.listRuns("acme", { repo: "App" })).runs[0].repoSlug).toBe("App");
    expect((await cloud.listSteps("acme", { repo: "App" }))[0].repoSlug).toBe("App");
    expect(server.seen).toContain("GET /v1/teams/acme/repos/app");
    expect(server.seen).toContain("GET /v1/teams/acme/runs?repo=app");
    // A GitHub-only repo is never asked of quod-server.
    expect((await cloud.getRepo("acme", "web"))?.summary).toMatchObject({ slug: "web", hasQuodData: false });
    expect(server.seen).not.toContain("GET /v1/teams/acme/repos/web");
    const idx = await cloud.getSearchIndex("acme");
    expect(idx.repos.map((r) => r.slug)).toEqual(["App", "web"]);
    expect(idx.runs.every((r) => r.repoSlug === "App")).toBe(true);
  });

  it("quod-server unreachable: GitHub's data alone, said so; never a thrown page", async () => {
    const cloud = composite(http(recorded(true).f));
    const ov = await cloud.getOverview("acme", "7d");
    expect(ov.quod).toMatchObject({ state: "unavailable" });
    expect(ov.repos.map((r) => [r.slug, r.hasQuodData])).toEqual([
      ["App", false],
      ["web", false],
    ]);
    expect(await cloud.getRepo("acme", "App")).toMatchObject({ summary: { slug: "App" }, quod: { state: "unavailable" } });
    expect(await cloud.listRuns("acme")).toMatchObject({ runs: [], quod: { state: "unavailable" } });
    expect(await cloud.getTeamHealth("acme")).toEqual({ wrongReplays: 0, spotChecks: 0, pausedSteps: 0, repos: 0 });
    expect((await cloud.getSearchIndex("acme")).repos).toHaveLength(2);
    const act = await cloud.getActivity("acme");
    expect(act).toMatchObject({ quod: { state: "unavailable" }, github: { state: "linked" } });
    expect(act.cursor).toMatch(/^gh-\d+$/);
    // Quod-only answers still fail, for the page's section card.
    await expect(cloud.getRun("acme", "App", 3)).rejects.toMatchObject({ name: "ApiUnavailableError" });
  });

  it("501s become what the page can show: no Quod runners, the built-in setup instructions", async () => {
    const cloud = composite(http(recorded().f));
    expect(await cloud.listRunners("acme")).toEqual([]);
    const setup = await cloud.getConnectSetup("acme");
    expect(setup.setupAction).toBe("quodci/setup@v1");
    expect(setup.firstRepo).toEqual({ slug: "App", fullName: "acme/App" });
    await expect(cloud.requestRerunWithoutQuod("acme", "App", 3)).rejects.toMatchObject({ name: "ApiNotAvailableError" });
  });

  it("no Quod server (QUOD_API unset): GitHub's repos, Quod's numbers 'appear once Quod runs'", async () => {
    const cloud = composite(new NoQuodCloudApi());
    const ov = await cloud.getOverview("acme", "7d");
    expect(ov.quod).toEqual({ state: "off" });
    expect(ov.repos.every((r) => r.hasQuodData === false)).toBe(true);
    expect(await cloud.getRun("acme", "App", 3)).toBeNull();
  });

  it("carries each repo's cached default-branch protection", async () => {
    await db.query(`INSERT INTO repo_protection (repo_id, installation_id, default_branch, state, classic, rulesets, checked_at) VALUES (7, 77, 'main', 'unprotected', false, false, now()), (8, 77, 'trunk', 'protected', true, false, now())`);
    const repos = await composite(http(recorded().f)).listRepos("acme");
    expect(repos.map((r) => [r.slug, r.protection?.state, r.protection?.branch])).toEqual([
      ["App", "unprotected", "main"],
      ["web", "protected", "trunk"],
    ]);
  });
});

describe("repo slug map", () => {
  it("matches by full name, case-insensitively; a repo only Quod knows keeps its slug unless GitHub's uses it", () => {
    const m = buildRepoMap(
      [
        { slug: "my-app-js", fullName: "acme/My.App.js" },
        { slug: "web", fullName: "other/web" },
        { slug: "solo", fullName: "acme/solo" },
      ],
      [
        { slug: "My.App.js", fullName: "acme/my.app.js" },
        { slug: "web", fullName: "acme/web" },
      ],
    );
    expect(m.toDash("my-app-js")).toBe("My.App.js");
    expect(m.toQuod("My.App.js")).toBe("my-app-js");
    expect(m.toDash("web")).toBe("other-web"); // GitHub's acme/web has "web"
    expect(m.toQuod("other-web")).toBe("web");
    expect(m.toDash("solo")).toBe("solo");
    expect(m.toQuod("web")).toBe("web"); // GitHub's own repo passes through (quod-server answers 404)
    const e = mapEvent(m, { id: "1", at: "x", type: "trust.changed", repoSlug: "my-app-js", key: "k", trust: { kind: "replaying" }, event: { kind: "proven", at: "x", runs: [1, 2] } } as LiveEvent);
    expect(e).toMatchObject({ repoSlug: "My.App.js" });
  });
});

describe("a GitHub run's steps, with what Quod did in each", () => {
  it("matches quod-server's job (GITHUB_JOB) and steps by name; GitHub's own steps stay unmarked", async () => {
    const run = (await http(recorded().f).getRun("acme", "app", 3))!;
    const jobs = [
      {
        id: 1001,
        name: "test",
        steps: [
          { number: 1, name: "Set up job" },
          { number: 2, name: "Run actions/checkout@v4" },
          { number: 3, name: "Test" },
          { number: 4, name: "Vet" },
          { number: 5, name: "Complete job" },
        ],
      },
    ];
    const m = quodMarks(jobs, run);
    expect([...m.steps.keys()]).toEqual(["1001:3", "1001:4"]);
    expect(m.steps.get("1001:3")).toMatchObject({ name: "Test", outcome: "replayed" });
    expect(m).toMatchObject({ replayed: 2, total: 2 });
    expect(m.savedSeconds).toBeCloseTo(33.99, 1);
  });

  it("matrix legs, 'Run <command>' names, and the step number when the runner sent it", () => {
    const run = {
      jobs: [
        {
          id: "test",
          name: "test",
          steps: [
            { name: "go test ./...", outcome: "replayed", seconds: 0.1, why: { kind: "replayed", savedSeconds: 10 } },
            { name: "go test ./...", outcome: "ran", seconds: 9, why: { kind: "network" } },
            { name: "lint", index: 6, outcome: "ran", seconds: 2, why: { kind: "network" } },
          ],
        },
      ],
    } as unknown as Parameters<typeof quodMarks>[1];
    const m = quodMarks(
      [
        { id: 1, name: "test (linux)", steps: [{ number: 3, name: "Run go test ./..." }, { number: 6, name: "Lint it" }] },
        { id: 2, name: "test (mac)", steps: [{ number: 3, name: "Run go test ./..." }] },
        { id: 3, name: "build", steps: [{ number: 3, name: "Run go test ./..." }] },
      ],
      run,
    );
    expect(m.steps.get("1:3")?.outcome).toBe("replayed");
    expect(m.steps.get("1:6")?.name).toBe("lint");
    expect(m.steps.get("2:3")?.outcome).toBe("ran");
    expect(m.steps.has("3:3")).toBe(false);
  });
});

import { NextRequest } from "next/server";
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { CompositeCloudApi } from "@/lib/api/composite";
import { FixtureCloudApi, newFixtureState } from "@/lib/api/fixture";
import type { Activity, LiveEvent, LiveJob } from "@/lib/api/types";
import { migratedDb, truncateAll } from "@/lib/db/testing";
import type { Db } from "@/lib/db/types";
import { delivery, jobPayload, REPO, runPayload, SECRET } from "@/lib/github/fixtures";
import { linkInstallation } from "@/lib/github/store";
import { handleWebhook } from "@/lib/github/webhook";
import { splitCursor } from "@/lib/live/cursor";
import { applyToJobs } from "@/lib/live/jobs";
import { SseDecoder, type SseMessage } from "@/lib/live/sse";

// End to end: a signed GitHub webhook -> Postgres (PGlite) -> the team's
// poller -> the SSE route -> the browser's job state (applyToJobs).

vi.mock("server-only", () => ({}));
vi.mock("next/server", async (orig) => ({ ...(await orig<typeof import("next/server")>()), connection: async () => {} }));
vi.mock("@/lib/github/lazy-reconcile", () => ({ lazyReconcileTeam: vi.fn(async () => undefined) }));
const findTeam = vi.fn();
vi.mock("@/lib/dal", () => ({ findTeam: (...a: unknown[]) => findTeam(...a) }));
let cloud: CompositeCloudApi;
vi.mock("@/lib/api", () => ({ api: () => cloud }));

const { GET } = await import("./route");

let db: Db;
const ORG = "org_acme";
const team = (slug: string) => ({ team: { slug, orgId: ORG }, admin: true });

const send = (event: string, payload: unknown) => handleWebhook({ db, secret: SECRET, ...delivery(event, payload) });

function composite(source: Partial<{ configured: boolean; db: () => Promise<Db> }> = {}) {
  return new CompositeCloudApi(new FixtureCloudApi(newFixtureState(), 0), {
    github: { configured: () => source.configured ?? true, appReady: () => true, db: source.db ?? (async () => db) },
    orgOf: async () => ORG,
    pollIntervalMs: 250,
  });
}

/** Reads SSE messages from a response until `until` holds or the time runs out. */
async function reader(res: Response) {
  const r = res.body!.getReader();
  const dec = new SseDecoder();
  const text = new TextDecoder();
  const got: SseMessage[] = [];
  const all: SseMessage[] = [];
  let pending: Promise<ReadableStreamReadResult<Uint8Array>> | null = null;
  return {
    got,
    async until(pred: (m: SseMessage[]) => boolean, ms = 8000) {
      const end = Date.now() + ms;
      while (!pred(got)) {
        if (Date.now() > end) throw new Error(`timed out; got ${got.map((m) => m.event).join(",")}; all ${all.map((m) => m.event).join(",")}`);
        // One read in flight at a time: a read that loses the race is kept for the next turn, never dropped.
        pending ??= r.read();
        const { value, done } = await Promise.race([pending, new Promise<{ value: undefined; done: false }>((ok) => setTimeout(() => ok({ value: undefined, done: false }), 200))]);
        if (value || done) pending = null;
        if (done) throw new Error("stream ended");
        if (value) {
          const ms = dec.push(text.decode(value, { stream: true }));
          got.push(...ms);
          all.push(...ms);
        }
      }
    },
    cancel: () => r.cancel(),
  };
}

const stream = (slug: string, headers: Record<string, string> = {}) =>
  GET(new NextRequest(`http://localhost/${slug}/events`, { headers }), { params: Promise.resolve({ team: slug }) } as never);

beforeAll(async () => {
  db = await migratedDb();
});
afterAll(async () => db.close());
beforeEach(async () => {
  await truncateAll(db);
  findTeam.mockReset();
  findTeam.mockImplementation(async (slug: string) => team(slug));
  cloud = composite();
});

async function link() {
  await db.tx((q) => linkInstallation(q, { installation: { id: 77, account: { id: 500, login: "acme", type: "Organization" } }, teamId: ORG, userId: "user_1", githubUserId: 1, repos: [REPO] }));
}

describe("GitHub jobs, live, end to end", { timeout: 30_000 }, () => {
  it("webhook -> poller -> SSE -> client state: queued, running, finished", async () => {
    await link();
    const res = await stream("acme");
    expect(res.status).toBe(200);
    const sse = await reader(res);
    await sse.until((m) => m.some((x) => x.event === "snapshot"));
    const snapshot = JSON.parse(sse.got.find((x) => x.event === "snapshot")!.data) as Activity;
    expect(snapshot.github).toEqual({ state: "linked", repos: 1, accounts: ["acme"] });
    expect(snapshot.jobs).toEqual([]);
    expect(splitCursor(snapshot.cursor).gh).toMatch(/^gh-\d+$/);

    // The browser's state, as the provider keeps it.
    let jobs: LiveJob[] = snapshot.jobs;
    const apply = () => {
      for (const m of sse.got.splice(0)) if (m.event?.startsWith("job.")) jobs = applyToJobs(jobs, JSON.parse(m.data) as LiveEvent);
    };

    await send("workflow_job", jobPayload("queued"));
    await sse.until((m) => m.some((x) => x.event === "job.queued"));
    apply();
    expect(jobs.map((j) => [j.id, j.status, j.repoName])).toEqual([["1234", "queued", "acme/api"]]);

    await send("workflow_run", runPayload("in_progress"));
    await send("workflow_job", jobPayload("in_progress"));
    await sse.until((m) => m.some((x) => x.event === "job.started"));
    apply();
    expect(jobs[0]).toMatchObject({ status: "in_progress", currentStep: { name: "Run tests" }, source: "github" });

    await send("workflow_job", jobPayload("completed"));
    await sse.until((m) => m.some((x) => x.event === "job.finished"));
    const last = sse.got.at(-1)!;
    apply();
    expect(jobs).toEqual([]);
    // Ids carry the GitHub position, so a reconnect resumes from it.
    expect(last.id).toMatch(/^gh-\d+$/);
    await sse.cancel();
  });

  it("a snapshot and its cursor lose nothing: a change made right after the snapshot still arrives", async () => {
    await link();
    await send("workflow_job", jobPayload("queued"));
    const snap = await cloud.getActivity("acme");
    expect(snap.jobs.map((j) => j.status)).toEqual(["queued"]);
    // Changes after the snapshot, before anyone subscribes.
    await send("workflow_job", jobPayload("completed"));
    const ac = new AbortController();
    const seen: LiveEvent[] = [];
    const it = cloud.subscribe("acme", { kind: "team" }, { lastEventId: snap.cursor, signal: ac.signal })[Symbol.asyncIterator]();
    const first = await it.next();
    seen.push(first.value as LiveEvent);
    ac.abort();
    let state = snap.jobs;
    for (const e of seen) state = applyToJobs(state, e);
    expect(seen[0]).toMatchObject({ type: "job.finished" });
    expect(state).toEqual([]);
  });

  it("resumes with Last-Event-ID after a reconnect", async () => {
    await link();
    await send("workflow_job", jobPayload("queued", { id: 1 }));
    const res = await stream("acme");
    const sse = await reader(res);
    await sse.until((m) => m.some((x) => x.event === "snapshot"));
    await send("workflow_job", jobPayload("queued", { id: 2 }));
    await sse.until((m) => m.some((x) => x.event === "job.queued"));
    const lastId = sse.got.find((x) => x.event === "job.queued")!.id!;
    await sse.cancel();

    await send("workflow_job", jobPayload("in_progress", { id: 2 })); // while disconnected
    const again = await reader(await stream("acme", { "last-event-id": lastId }));
    await again.until((m) => m.some((x) => x.event === "job.started"));
    const started = JSON.parse(again.got.find((x) => x.event === "job.started")!.data) as LiveEvent;
    expect(started.type === "job.started" && started.job.id).toBe("2");
    await again.cancel();
  });

  it("an unlinked team gets an empty snapshot that says so, and a team's jobs stay its own", async () => {
    await send("workflow_job", jobPayload("queued")); // installation 77 exists but is linked to nobody
    const a = await cloud.getActivity("acme");
    expect(a.github).toEqual({ state: "unlinked", canInstall: true });
    expect(a.jobs).toEqual([]);
  });

  it("lists the installation's repos with their latest run before Quod has data", async () => {
    await link();
    await send("workflow_run", runPayload("completed"));
    const ov = await cloud.getOverview("acme", "7d");
    expect(ov.repos).toEqual([
      expect.objectContaining({ slug: "api", fullName: "acme/api", hasQuodData: false, github: { latestRun: expect.objectContaining({ status: "completed", conclusion: "success" }) } }),
    ]);
    expect(await cloud.getRepo("acme", "api")).toMatchObject({ summary: { slug: "api", hasQuodData: false }, latestRun: null });
  });
});

describe("no database, no crash", { timeout: 30_000 }, () => {
  it("without DATABASE_URL: pages get the inner data and a not-configured notice", async () => {
    cloud = composite({ configured: false });
    const a = await cloud.getActivity("acme");
    expect(a.github).toEqual({ state: "not-configured" });
    expect((await cloud.getOverview("acme", "7d")).repos).toEqual([]);
  });

  it("with a database that does not answer: an 'unavailable' notice, and the stream still opens", async () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    cloud = composite({ db: () => Promise.reject(new Error("ECONNREFUSED")) });
    expect((await cloud.getActivity("acme")).github).toEqual({ state: "unavailable" });
    expect((await cloud.getOverview("acme", "7d")).repos).toEqual([]);
    const res = await stream("acme");
    expect(res.status).toBe(200);
    const sse = await reader(res);
    await sse.until((m) => m.some((x) => x.event === "snapshot"));
    await sse.cancel();
  });

  it("the sample team keeps the simulator", async () => {
    const a = await cloud.getActivity("quodci");
    expect(a.simulated).toBe(true);
    expect(a.github).toBeUndefined();
  });
});

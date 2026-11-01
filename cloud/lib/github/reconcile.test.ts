import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { migratedDb, truncateAll } from "@/lib/db/testing";
import type { Db } from "@/lib/db/types";
import type { HookDelivery } from "./api";
import { cronAuthorized } from "./cron";
import { delivery, jobPayload, REPO, runPayload, SECRET } from "./fixtures";
import { activePollers, runningJobs, subscribeTeamJobs } from "./live";
import { claimReconcile, failedDeliveries, reconcile, reconcileDaily, reconcileTeamIfDue, staleRuns, type ReconcileGitHub } from "./reconcile";
import { linkInstallation } from "./store";
import { teamIdOf } from "./team";
import { handleWebhook } from "./webhook";

// Review M2 (GitHub does not redeliver failed webhooks), reworked for Vercel
// Hobby: a daily cron, plus a lazy per-team pass while someone watches,
// throttled across instances by reconcile_state.

let db: Db;
const TEAM = teamIdOf({ team: { orgId: "org_acme" } });
const OTHER = teamIdOf({ team: { orgId: "org_other" } });
const OTHER_REPO = { ...REPO, id: 7000, name: "x", full_name: "other/x" };
const send = (event: string, payload: unknown, id?: string) => handleWebhook({ db, secret: SECRET, ...delivery(event, payload, id) });
const d = (id: number, guid: string, status: number, minAgo: number, installation = 77): HookDelivery => ({
  id,
  guid,
  status_code: status,
  delivered_at: new Date(Date.now() - minAgo * 60_000).toISOString(),
  redelivery: false,
  event: "workflow_job",
  installation_id: installation,
});

const link = (id: number, teamId: string, repo = REPO) =>
  db.tx((q) => linkInstallation(q, { installation: { id, account: { id: 500, login: "acme", type: "Organization" } }, teamId, userId: "user_1", githubUserId: 1, repos: [repo] }));

beforeAll(async () => {
  db = await migratedDb();
});
afterAll(async () => db.close());
beforeEach(async () => {
  await truncateAll(db);
  await link(77, TEAM);
});

const fakeGh = (o: Partial<ReconcileGitHub> = {}) =>
  ({
    listFailedDeliveries: vi.fn(async () => [] as HookDelivery[]),
    redeliver: vi.fn(async () => true),
    getRun: vi.fn(async () => null),
    listRunJobs: vi.fn(async () => []),
    ...o,
  }) as { [K in keyof ReconcileGitHub]: ReturnType<typeof vi.fn> } & ReconcileGitHub;

const staleAll = async () => {
  await db.query("UPDATE workflow_runs SET row_updated_at = now() - interval '20 minutes'");
  await db.query("UPDATE workflow_jobs SET row_updated_at = now() - interval '20 minutes'");
};

describe("failedDeliveries", () => {
  it("picks GUIDs with no successful attempt, newest failed attempt of each", () => {
    const list = [d(1, "a", 500, 30), d(2, "a", 502, 10), d(3, "b", 500, 20), d(4, "b", 200, 5), d(5, "c", 200, 5), d(6, "e", 404, 1)];
    expect(failedDeliveries(list).map((x) => x.id)).toEqual([2, 6]);
  });
});

describe("claimReconcile (the cross-instance throttle)", () => {
  it("lets exactly one of several concurrent callers run, then none until the interval passed", async () => {
    const wins = await Promise.all([1, 2, 3, 4].map(() => claimReconcile(db, "team:org_acme", 15)));
    expect(wins.filter(Boolean)).toHaveLength(1);
    expect(await claimReconcile(db, "team:org_acme", 15)).toBe(false);
    expect(await claimReconcile(db, "team:org_other", 15)).toBe(true); // per team
    await db.query("UPDATE reconcile_state SET last_run_at = now() - interval '16 minutes' WHERE key = 'team:org_acme'");
    expect(await claimReconcile(db, "team:org_acme", 15)).toBe(true);
    expect(await claimReconcile(db, "team:org_acme", 15)).toBe(false);
  });
});

describe("reconcile", () => {
  it("redelivers failures Quod never stored, and skips ones that landed after all", async () => {
    await send("workflow_job", jobPayload("queued"), "guid-landed");
    const gh = fakeGh({ listFailedDeliveries: vi.fn(async () => [d(9, "guid-lost", 500, 5), d(10, "guid-landed", 502, 6)]) });
    const r = await reconcileDaily(db, gh);
    expect(gh.redeliver).toHaveBeenCalledTimes(1);
    expect(gh.redeliver).toHaveBeenCalledWith(9);
    expect(r).toMatchObject({ failuresSeen: 1, redelivered: 1 });
  });

  it("heals a job stuck in_progress whose completion was lost", async () => {
    await send("workflow_run", runPayload("in_progress"));
    await send("workflow_job", jobPayload("in_progress"));
    await staleAll();
    expect((await staleRuns(db, 15, 20)).map((r) => r.run_id)).toEqual([555]);
    const gh = fakeGh({
      getRun: vi.fn(async () => ({ ...runPayload("completed", { updated: new Date().toISOString() }).workflow_run })),
      listRunJobs: vi.fn(async () => [jobPayload("completed").workflow_job]),
    });
    const r = await reconcile({ db, gh, windowMs: 3_600_000 });
    expect(r).toMatchObject({ swept: 1, jobsHealed: 1, errors: 0, pruned: null });
    expect(gh.getRun).toHaveBeenCalledWith(77, "acme/api", 555);
    expect((await db.query("SELECT status FROM workflow_jobs WHERE id = 1234"))[0]).toEqual({ status: "completed" });
    expect(await runningJobs(TEAM, db)).toEqual([]);
    expect(await staleRuns(db, 15, 20)).toEqual([]);
  });

  it("rotates a healthy long run to the back (reconciled_at), and drops a run GitHub deleted", async () => {
    await send("workflow_job", jobPayload("in_progress", { id: 1, runId: 601 }));
    await send("workflow_job", jobPayload("in_progress", { id: 2, runId: 602 }));
    await staleAll();
    const gh = fakeGh({
      getRun: vi.fn(async (_i: number, _n: string, runId: number) => (runId === 601 ? { ...runPayload("in_progress", { id: 601, updated: new Date().toISOString() }).workflow_run } : null)),
      listRunJobs: vi.fn(async () => [jobPayload("in_progress", { id: 1, runId: 601 }).workflow_job]),
    });
    expect((await reconcile({ db, gh, windowMs: 3_600_000 })).swept).toBe(2);
    expect((await db.query("SELECT id FROM workflow_jobs ORDER BY id")).map((r) => r.id)).toEqual([1]);
    expect(await staleRuns(db, 15, 20)).toEqual([]);
  });

  it("ignores repos of installations no team has linked", async () => {
    await send("workflow_job", jobPayload("in_progress", { id: 3, runId: 700, installation: 88, repo: OTHER_REPO }));
    await staleAll();
    expect(await staleRuns(db, 15, 20)).toEqual([]);
  });

  it("a team's lazy pass touches only that team's deliveries and runs", async () => {
    await link(88, OTHER, OTHER_REPO);
    await send("workflow_job", jobPayload("in_progress", { id: 1, runId: 801 }));
    await send("workflow_job", jobPayload("in_progress", { id: 2, runId: 802, installation: 88, repo: OTHER_REPO }));
    await staleAll();
    const gh = fakeGh({ listFailedDeliveries: vi.fn(async () => [d(1, "mine", 500, 5, 77), d(2, "theirs", 500, 5, 88), d(3, "unknown", 500, 5, 99)]) });
    const r = await reconcileTeamIfDue(db, gh, TEAM);
    expect(gh.redeliver.mock.calls).toEqual([[1]]);
    expect(gh.getRun.mock.calls.map((c) => c[2])).toEqual([801]);
    expect(r).toMatchObject({ redelivered: 1, swept: 1, pruned: null });
    // Throttled now, for this team only.
    expect(await reconcileTeamIfDue(db, gh, TEAM)).toBeNull();
    expect(await reconcileTeamIfDue(db, gh, OTHER)).not.toBeNull();
  });

  it("the daily pass covers every team and prunes", async () => {
    const r = await reconcileDaily(db, fakeGh());
    expect(r.pruned).toEqual({ deliveries: 0, runs: 0, jobs: 0 });
  });
});

describe("lazy reconcile from the live poller", () => {
  it("runs in the background while someone watches, at most once per interval per poller, and stops with the poller", async () => {
    const calls: string[] = [];
    let release!: () => void;
    const slow = new Promise<void>((r) => (release = r));
    const hook = vi.fn(async (teamId: string) => {
      calls.push(teamId);
      await slow; // a slow reconcile must not hold up polling
    });
    const events: string[] = [];
    const un1 = subscribeTeamJobs(TEAM, (e) => events.push(e.job.id), { db, intervalMs: 250, reconcile: hook, reconcileEveryMs: 1000 });
    const un2 = subscribeTeamJobs(TEAM, () => {}, { db, intervalMs: 250, reconcile: hook, reconcileEveryMs: 1000 });
    await new Promise((r) => setTimeout(r, 400));
    expect(calls).toEqual(["org_acme"]); // two viewers, one poller, one call
    await send("workflow_job", jobPayload("in_progress", { id: 41 }));
    await new Promise((r) => setTimeout(r, 500));
    expect(events).toEqual(["41"]); // the stream kept flowing while reconcile was still pending
    await new Promise((r) => setTimeout(r, 700));
    expect(calls.length).toBe(2); // once more after reconcileEveryMs
    un1();
    un2();
    await new Promise((r) => setTimeout(r, 20));
    expect(activePollers(db)).toBe(0);
    const n = calls.length;
    await new Promise((r) => setTimeout(r, 1200));
    expect(calls.length).toBe(n); // nobody watching: no more reconciles
    release();
  });
});

describe("cron auth", () => {
  it("needs CRON_SECRET, and the exact bearer header", () => {
    const s = "s".repeat(32);
    expect(cronAuthorized(`Bearer ${s}`, s)).toBe(true);
    expect(cronAuthorized(`Bearer ${s}x`, s)).toBe(false);
    expect(cronAuthorized(s, s)).toBe(false);
    expect(cronAuthorized(null, s)).toBe(false);
    expect(cronAuthorized("Bearer ", undefined)).toBe(false);
    expect(cronAuthorized("Bearer short", "short")).toBe(false);
  });
});

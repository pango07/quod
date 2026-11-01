import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { migratedDb, truncateAll } from "@/lib/db/testing";
import type { Db } from "@/lib/db/types";
import type { HookDelivery } from "./api";
import { cronAuthorized } from "./cron";
import { delivery, jobPayload, REPO, runPayload, SECRET } from "./fixtures";
import { runningJobs } from "./live";
import { failedDeliveries, reconcile, staleRuns, type ReconcileGitHub } from "./reconcile";
import { linkInstallation } from "./store";
import { teamIdOf } from "./team";
import { handleWebhook } from "./webhook";

// Review M2: GitHub does not redeliver failed webhooks by itself.

let db: Db;
const TEAM = teamIdOf({ team: { orgId: "org_acme" } });
const send = (event: string, payload: unknown) => handleWebhook({ db, secret: SECRET, ...delivery(event, payload) });
const d = (id: number, guid: string, status: number, minAgo: number): HookDelivery => ({ id, guid, status_code: status, delivered_at: new Date(Date.now() - minAgo * 60_000).toISOString(), redelivery: false, event: "workflow_job" });

beforeAll(async () => {
  db = await migratedDb();
});
afterAll(async () => db.close());
beforeEach(async () => {
  await truncateAll(db);
  await db.tx((q) => linkInstallation(q, { installation: { id: 77, account: { id: 500, login: "acme", type: "Organization" } }, teamId: TEAM, userId: "user_1", githubUserId: 1, repos: [REPO] }));
});

const fakeGh = (o: Partial<ReconcileGitHub> = {}): { [K in keyof ReconcileGitHub]: ReturnType<typeof vi.fn> } =>
  ({
    listDeliveries: vi.fn(async () => [] as HookDelivery[]),
    redeliver: vi.fn(async () => true),
    getRun: vi.fn(async () => null),
    listRunJobs: vi.fn(async () => []),
    ...o,
  }) as never;

describe("failedDeliveries", () => {
  it("picks GUIDs with no successful attempt, newest failed attempt of each", () => {
    const list = [d(1, "a", 500, 30), d(2, "a", 502, 10), d(3, "b", 500, 20), d(4, "b", 200, 5), d(5, "c", 200, 5), d(6, "e", 404, 1)];
    expect(failedDeliveries(list).map((x) => x.id)).toEqual([2, 6]);
  });
});

describe("reconcile", () => {
  it("redelivers failures without touching the database when nothing else is needed", async () => {
    const gh = fakeGh({ listDeliveries: vi.fn(async () => [d(9, "x", 500, 5)]) });
    const dbOpen = vi.fn(async () => db);
    const quiet = await reconcile({ db: dbOpen, gh: fakeGh() as never });
    expect(quiet).toMatchObject({ deliveriesSeen: 0, redelivered: 0, swept: 0, pruned: null });
    expect(dbOpen).not.toHaveBeenCalled(); // Neon stays asleep
    const r = await reconcile({ db: dbOpen, gh: gh as never });
    expect(gh.redeliver).toHaveBeenCalledWith(9);
    expect(r.redelivered).toBe(1);
  });

  it("heals a job stuck in_progress whose completion was lost", async () => {
    await send("workflow_run", runPayload("in_progress"));
    await send("workflow_job", jobPayload("in_progress"));
    await db.query("UPDATE workflow_runs SET row_updated_at = now() - interval '20 minutes'");
    await db.query("UPDATE workflow_jobs SET row_updated_at = now() - interval '20 minutes'");
    expect((await staleRuns(db, 15, 20)).map((r) => r.run_id)).toEqual([555]);

    const done = jobPayload("completed").workflow_job;
    const gh = fakeGh({
      listDeliveries: vi.fn(async () => [d(1, "ok", 200, 1)]),
      getRun: vi.fn(async () => ({ ...runPayload("completed", { updated: new Date().toISOString() }).workflow_run })),
      listRunJobs: vi.fn(async () => [done]),
    });
    const r = await reconcile({ db: async () => db, gh: gh as never });
    expect(r).toMatchObject({ swept: 1, jobsHealed: 1, errors: 0 });
    expect(gh.getRun).toHaveBeenCalledWith(77, "acme/api", 555);
    expect((await db.query("SELECT status FROM workflow_jobs WHERE id = 1234"))[0]).toEqual({ status: "completed" });
    expect((await db.query("SELECT status FROM workflow_runs WHERE id = 555"))[0]).toEqual({ status: "completed" });
    expect(await runningJobs(TEAM, db)).toEqual([]);
    expect(await staleRuns(db, 15, 20)).toEqual([]);
  });

  it("rotates a healthy long run to the back (reconciled_at), and drops a run GitHub deleted", async () => {
    await send("workflow_job", jobPayload("in_progress", { id: 1, runId: 601 }));
    await send("workflow_job", jobPayload("in_progress", { id: 2, runId: 602 }));
    await db.query("UPDATE workflow_jobs SET row_updated_at = now() - interval '20 minutes'");
    const still = jobPayload("in_progress", { id: 1, runId: 601 }).workflow_job;
    const gh = fakeGh({
      listDeliveries: vi.fn(async () => [d(1, "ok", 200, 1)]),
      getRun: vi.fn(async (_i: number, _n: string, runId: number) => (runId === 601 ? { ...runPayload("in_progress", { id: 601, updated: new Date().toISOString() }).workflow_run } : null)),
      listRunJobs: vi.fn(async () => [still]),
    });
    expect((await reconcile({ db: async () => db, gh: gh as never })).swept).toBe(2);
    expect((await db.query("SELECT id FROM workflow_jobs ORDER BY id")).map((r) => r.id)).toEqual([1]);
    // 601 is still running on GitHub: checked now, so not stale again yet.
    expect(await staleRuns(db, 15, 20)).toEqual([]);
  });

  it("ignores repos of installations no team has linked", async () => {
    const other = { ...REPO, id: 7000, name: "x", full_name: "other/x" };
    await send("workflow_job", jobPayload("in_progress", { id: 3, runId: 700, installation: 88, repo: other }));
    await db.query("UPDATE workflow_jobs SET row_updated_at = now() - interval '20 minutes'");
    expect(await staleRuns(db, 15, 20)).toEqual([]);
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

import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { migratedDb, truncateAll } from "@/lib/db/testing";
import type { Db } from "@/lib/db/types";
import { delivery, installationPayload, jobPayload, REPO, runPayload, SECRET } from "./fixtures";
import { activePollers, jobById, jobEventFromPayload, nextDelay, onTeamJobChange, queuedJobs, recentRuns, runningJobs, subscribeTeamJobs, teamInstallations } from "./live";
import { disconnectInstallation, linkInstallation, pruneOld } from "./store";
import { teamIdOf, type TeamId } from "./team";
import { handleWebhook } from "./webhook";

// The pollers' default lazy-reconcile hook needs the App's credentials; reconcile.test.ts covers it.
const lazyReconcileTeam = vi.hoisted(() => vi.fn(async () => undefined));
vi.mock("./lazy-reconcile", () => ({ lazyReconcileTeam }));

let db: Db;
const TEAM = teamIdOf({ team: { orgId: "org_acme" } });
const OTHER = teamIdOf({ team: { orgId: "org_other" } });

const send = (event: string, payload: unknown, id?: string) => handleWebhook({ db, secret: SECRET, ...delivery(event, payload, id) });

async function link(installationId = 77, teamId: TeamId = TEAM) {
  return db.tx((q) => linkInstallation(q, { installation: { id: installationId, account: { id: 500, login: "acme", type: "Organization" } }, teamId, userId: "user_1", githubUserId: 1, repos: [REPO] }));
}

const jobRow = async (id = 1234) => (await db.query<{ status: string; conclusion: string | null; steps: unknown[]; version: number }>("SELECT status, conclusion, steps, version FROM workflow_jobs WHERE id = $1", [id]))[0];

beforeAll(async () => {
  db = await migratedDb();
});
afterAll(async () => db.close());
beforeEach(async () => truncateAll(db));

describe("webhook receiver", () => {
  it("refuses a bad or missing signature and stores nothing", async () => {
    const d = delivery("workflow_job", jobPayload("queued"));
    d.headers.set("x-hub-signature-256", "sha256=" + "0".repeat(64));
    expect((await handleWebhook({ db, secret: SECRET, ...d })).status).toBe(401);
    d.headers.delete("x-hub-signature-256");
    expect((await handleWebhook({ db, secret: SECRET, ...d })).status).toBe(401);
    expect((await handleWebhook({ db, secret: "", ...delivery("workflow_job", jobPayload("queued")) })).status).toBe(500);
    expect(await db.query("SELECT * FROM deliveries")).toEqual([]);
    expect(await db.query("SELECT * FROM workflow_jobs")).toEqual([]);
  });

  it("refuses a missing delivery id or event, and a body that is not JSON", async () => {
    const d = delivery("workflow_job", jobPayload("queued"));
    d.headers.delete("x-github-delivery");
    expect((await handleWebhook({ db, secret: SECRET, ...d })).status).toBe(400);
    const notJson = new TextEncoder().encode("not json");
    const { signBody } = await import("./signature");
    const h = new Headers({ "x-github-event": "ping", "x-github-delivery": "abc", "x-hub-signature-256": signBody(SECRET, notJson) });
    expect((await handleWebhook({ db, secret: SECRET, headers: h, body: notJson })).status).toBe(400);
  });

  it("applies a delivery once: a redelivery with the same X-GitHub-Delivery is a no-op", async () => {
    const id = "11111111-2222-4333-8444-555555555555";
    const first = await send("workflow_job", jobPayload("in_progress"), id);
    expect(first.body).toEqual({ ok: true });
    const v1 = (await jobRow()).version;
    const again = await send("workflow_job", jobPayload("completed"), id);
    expect(again.body).toEqual({ ok: true, duplicate: true });
    expect((await jobRow()).status).toBe("in_progress");
    expect((await jobRow()).version).toBe(v1);
    expect(await db.query("SELECT id, event FROM deliveries")).toEqual([{ id, event: "workflow_job" }]);
  });

  it("does not mark a delivery seen when applying it fails (so a redelivery works)", async () => {
    const id = "99999999-2222-4333-8444-555555555555";
    // A database that fails on the job write, after the delivery row was inserted in the same transaction.
    const flaky: Db = {
      ...db,
      tx: (fn) =>
        db.tx((q) =>
          fn({
            exec: (t) => q.exec(t),
            query: (t, p) => (t.includes("INSERT INTO workflow_jobs") ? Promise.reject(new Error("connection reset")) : q.query(t, p)),
          }),
        ),
    };
    await expect(handleWebhook({ db: flaky, secret: SECRET, ...delivery("workflow_job", jobPayload("queued"), id) })).rejects.toThrow("connection reset");
    expect(await db.query("SELECT * FROM deliveries")).toEqual([]);
    expect((await send("workflow_job", jobPayload("queued"), id)).body).toEqual({ ok: true });
  });

  it("skips (not fails) a workflow_job without run_id or name (M2 guard)", async () => {
    for (const field of ["run_id", "name"] as const) {
      const p = jobPayload("queued") as unknown as { workflow_job: Record<string, unknown> };
      delete p.workflow_job[field];
      expect((await send("workflow_job", p)).body).toEqual({ ok: true, ignored: true });
    }
    expect(await db.query("SELECT * FROM workflow_jobs")).toEqual([]);
  });

  it("ranks an unknown GitHub status as not started, and logs it (m3)", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const p = jobPayload("queued") as unknown as { workflow_job: { status: string } };
    p.workflow_job.status = "something_new";
    await send("workflow_job", p);
    expect((await db.query("SELECT status_rank FROM workflow_jobs WHERE id = 1234"))[0]).toEqual({ status_rank: 1 });
    expect(warn).toHaveBeenCalledWith("[quod-cloud] unknown github status", { status: "something_new" });
    // A real completion still wins over it.
    await send("workflow_job", jobPayload("completed"));
    expect((await jobRow()).status).toBe("completed");
    warn.mockRestore();
  });

  it("records and ignores events it does not handle", async () => {
    expect((await send("ping", { zen: "Keep it logically awesome." })).body).toEqual({ ok: true, ignored: true });
    expect((await db.query("SELECT event FROM deliveries"))[0]).toEqual({ event: "ping" });
  });
});

describe("monotonic status (out-of-order deliveries)", () => {
  it("never lets a late in_progress or queued overwrite completed", async () => {
    await send("workflow_job", jobPayload("completed", { conclusion: "failure" }));
    const done = await jobRow();
    expect(done.status).toBe("completed");
    expect((await send("workflow_job", jobPayload("in_progress"))).versions).toEqual([]);
    expect((await send("workflow_job", jobPayload("queued"))).versions).toEqual([]);
    const after = await jobRow();
    expect(after).toEqual(done);
    expect(after.conclusion).toBe("failure");
    expect(after.steps).toHaveLength(2);
  });

  it("moves forward in any arrival order and ends at the latest state", async () => {
    const orders = [
      ["queued", "in_progress", "completed"],
      ["completed", "queued", "in_progress"],
      ["in_progress", "completed", "queued"],
      ["in_progress", "queued", "completed"],
    ] as const;
    let id = 1;
    for (const order of orders) {
      id += 1;
      for (const s of order) await send("workflow_job", jobPayload(s, { id }));
      expect((await jobRow(id)).status).toBe("completed");
    }
  });

  it("orders deliveries of the same status by the payload's timestamps", async () => {
    const later = new Date(Date.now() - 2 * 60_000).toISOString();
    const earlier = new Date(Date.now() - 20 * 60_000).toISOString();
    await send("workflow_job", jobPayload("in_progress", { t: later }));
    const v = (await jobRow()).version;
    expect((await send("workflow_job", jobPayload("in_progress", { t: earlier }))).versions).toEqual([]);
    expect((await jobRow()).version).toBe(v);
  });

  it("lets a re-run (a new attempt) move a completed run back to queued, but not a late delivery of the old attempt", async () => {
    await send("workflow_run", runPayload("completed", { attempt: 1, updated: "2026-11-01T10:05:00Z" }));
    await send("workflow_run", runPayload("queued", { attempt: 2, updated: "2026-11-01T11:00:00Z" }));
    const [r] = await db.query<{ status: string; run_attempt: number }>("SELECT status, run_attempt FROM workflow_runs WHERE id = 555");
    expect(r).toEqual({ status: "queued", run_attempt: 2 });
    await send("workflow_run", runPayload("in_progress", { attempt: 1, updated: "2026-11-01T10:02:00Z" }));
    expect((await db.query("SELECT status, run_attempt FROM workflow_runs WHERE id = 555"))[0]).toEqual({ status: "queued", run_attempt: 2 });
  });

  it("re-announces a job that arrived before its run, once the run's number is known", async () => {
    await link();
    await send("workflow_job", jobPayload("in_progress"));
    const before = await jobById(TEAM, 1234, db);
    expect(before?.runNumber).toBe(0);
    const v = (await jobRow()).version;
    await send("workflow_run", runPayload("in_progress"));
    expect((await jobRow()).version).toBeGreaterThan(v);
    expect((await jobById(TEAM, 1234, db))?.runNumber).toBe(42);
    expect((await jobById(TEAM, 1234, db))?.actor).toBe("jamie");
  });
});

describe("installations", () => {
  it("unlinks an installation when it is deleted on GitHub", async () => {
    await send("installation", installationPayload("created"));
    await link();
    await send("workflow_job", jobPayload("in_progress"));
    expect(await runningJobs(TEAM, db)).toHaveLength(1);
    await send("installation", installationPayload("deleted"));
    const [i] = await db.query<{ team_id: string | null; deleted_at: Date | null }>("SELECT team_id, deleted_at FROM installations WHERE id = 77");
    expect(i.team_id).toBeNull();
    expect(i.deleted_at).toBeInstanceOf(Date);
    expect(await runningJobs(TEAM, db)).toEqual([]);
  });

  it("keeps the team link when webhooks update the installation, and tracks repos added and removed", async () => {
    await link();
    await send("installation", installationPayload("new_permissions_accepted"));
    expect((await db.query("SELECT team_id FROM installations WHERE id = 77"))[0]).toEqual({ team_id: "org_acme" });
    await send("installation_repositories", { action: "added", installation: { id: 77 }, repository_selection: "selected", repositories_added: [{ id: 9002, name: "web", full_name: "acme/web", private: false }], repositories_removed: [{ id: REPO.id, name: "api", full_name: "acme/api" }] });
    const repos = await db.query<{ id: number; removed: boolean }>("SELECT id, removed_at IS NOT NULL AS removed FROM repos ORDER BY id");
    expect(repos).toEqual([
      { id: 9001, removed: true },
      { id: 9002, removed: false },
    ]);
    // A removed repo's jobs are no longer shown.
    await send("workflow_job", jobPayload("in_progress"));
    expect(await runningJobs(TEAM, db)).toEqual([]);
  });

  it("refuses to link an installation already linked to another team", async () => {
    expect(await link(77, TEAM)).toBe("linked");
    expect(await link(77, OTHER)).toBe("linked-elsewhere");
    expect(await link(77, TEAM)).toBe("linked");
    expect((await db.query("SELECT team_id FROM installations WHERE id = 77"))[0]).toEqual({ team_id: "org_acme" });
  });
});

describe("live queries and events", () => {
  it("scope every query to the team's linked installations", async () => {
    await link(77, TEAM);
    await send("workflow_run", runPayload("in_progress"));
    await send("workflow_job", jobPayload("in_progress", { id: 1 }));
    await send("workflow_job", jobPayload("queued", { id: 2 }));
    // Another org's installation, not linked to TEAM.
    const theirs = { ...REPO, id: 7000, name: "secret", full_name: "other/secret" };
    await send("workflow_job", jobPayload("in_progress", { id: 3, installation: 88, repo: theirs }));

    expect((await runningJobs(TEAM, db)).map((j) => j.id)).toEqual(["1"]);
    expect((await queuedJobs(TEAM, db)).map((j) => j.id)).toEqual(["2"]);
    expect(await jobById(TEAM, 3, db)).toBeNull();
    expect(await runningJobs(OTHER, db)).toEqual([]);
    expect((await recentRuns(TEAM, null, { db })).map((r) => r.runNumber)).toEqual([42]);
    expect((await recentRuns(TEAM, "acme/api", { db })).length).toBe(1);
    expect((await recentRuns(TEAM, "api", { db })).length).toBe(1);
    expect(await recentRuns(TEAM, "other/secret", { db })).toEqual([]);
    expect(await recentRuns(OTHER, null, { db })).toEqual([]);
  });

  it("maps a stored job to the live job shape", async () => {
    await link();
    await send("workflow_run", runPayload("in_progress"));
    await send("workflow_job", jobPayload("in_progress"));
    const job = await jobById(TEAM, "1234", db);
    expect(job).toMatchObject({
      id: "1234",
      repoSlug: "api",
      repoName: "acme/api",
      runNumber: 42,
      workflow: "CI",
      job: "test",
      branch: "main",
      sha: "a".repeat(40),
      actor: "jamie",
      runner: "ubuntu-latest",
      status: "in_progress",
      currentStep: { index: 2, name: "Run tests" },
      stepsTotal: 2,
      source: "github",
      runId: 555,
      labels: ["ubuntu-latest"],
    });
    expect(job?.startedAt).toBeTruthy();
    expect(job?.finishedAt).toBeUndefined();
    expect(job?.githubUrl).toContain("/job/1234");
  });

  it("maps a raw payload to the same event union", () => {
    const e = jobEventFromPayload(jobPayload("completed", { conclusion: "timed_out" }) as never, { version: 7 });
    expect(e).toMatchObject({ id: "gh-7", type: "job.finished", job: { status: "completed", conclusion: "failure", githubConclusion: "timed_out", runNumber: 0, actor: "octocat" } });
    expect(jobEventFromPayload(jobPayload("queued") as never, { version: 8 })?.type).toBe("job.queued");
  });

  it("onTeamJobChange yields each change once, in order, and only the team's", async () => {
    await link();
    const ctrl = new AbortController();
    const feed = onTeamJobChange(TEAM, { signal: ctrl.signal, intervalMs: 250, db });
    const got: string[] = [];
    const reading = (async () => {
      for await (const e of feed) {
        got.push(`${e.type}:${e.job.id}`);
        if (got.length === 3) ctrl.abort();
      }
    })();
    await new Promise((r) => setTimeout(r, 100));
    await send("workflow_job", jobPayload("queued", { id: 10 }));
    await send("workflow_job", jobPayload("in_progress", { id: 10 }));
    await send("workflow_job", jobPayload("in_progress", { id: 99, installation: 88, repo: { ...REPO, id: 7000, full_name: "other/x", name: "x" } }));
    await new Promise((r) => setTimeout(r, 400));
    await send("workflow_job", jobPayload("completed", { id: 10 }));
    await send("workflow_job", jobPayload("completed", { id: 10 })); // redelivery content, new delivery id: no change, no event
    const timeout = setTimeout(() => ctrl.abort(), 3000);
    await reading;
    clearTimeout(timeout);
    // The first poll sees the latest state of job 10 (started); the queued step in between may be folded.
    expect(got[got.length - 1]).toBe("job.finished:10");
    expect(got.every((g) => g.endsWith(":10"))).toBe(true);
    expect(new Set(got).size).toBe(got.length);
  });

  it("onTeamJobChange resumes after a Last-Event-ID", async () => {
    await link();
    await send("workflow_job", jobPayload("in_progress", { id: 20 }));
    const v = (await jobRow(20)).version;
    await send("workflow_job", jobPayload("completed", { id: 21 }));
    const ctrl = new AbortController();
    const out = [];
    for await (const e of onTeamJobChange(TEAM, { signal: ctrl.signal, intervalMs: 250, after: `gh-${v}`, db })) {
      out.push(e);
      ctrl.abort();
    }
    expect(out.map((e) => e.job.id)).toEqual(["21"]);
  });
});

describe("review fixes", () => {
  const inst = (action: string, updated: string, extra: Record<string, unknown> = {}) => ({
    action,
    installation: { id: 77, account: { id: 500, login: "acme", type: "Organization" }, repository_selection: "selected", updated_at: updated, ...extra },
  });

  it("m2: a late installation delivery never undoes a newer one (suspend / unsuspend)", async () => {
    await link();
    await send("installation", inst("unsuspend", "2026-11-01T12:00:00Z"));
    await send("installation", inst("suspend", "2026-11-01T11:00:00Z", { suspended_at: "2026-11-01T11:00:00Z" })); // older, arrives late
    expect((await db.query("SELECT suspended_at FROM installations WHERE id = 77"))[0]).toEqual({ suspended_at: null });
    await send("installation", inst("suspend", "2026-11-01T13:00:00Z", { suspended_at: "2026-11-01T13:00:00Z" }));
    expect((await db.query<{ s: boolean }>("SELECT suspended_at IS NOT NULL AS s FROM installations WHERE id = 77"))[0].s).toBe(true);
  });

  it("m2: a late 'added' never revives a repo a newer delivery removed", async () => {
    await link();
    const repos = (action: "added" | "removed", updated: string) => ({
      action,
      installation: { id: 77, updated_at: updated },
      repository_selection: "selected",
      [action === "added" ? "repositories_added" : "repositories_removed"]: [{ id: 9002, name: "web", full_name: "acme/web", private: false }],
    });
    await send("installation_repositories", repos("added", "2026-11-01T10:00:00Z"));
    await send("installation_repositories", repos("removed", "2026-11-01T12:00:00Z"));
    await send("installation_repositories", repos("added", "2026-11-01T11:00:00Z")); // older, late
    expect((await db.query<{ removed: boolean }>("SELECT removed_at IS NOT NULL AS removed FROM repos WHERE id = 9002"))[0].removed).toBe(true);
    await send("installation_repositories", repos("added", "2026-11-01T13:00:00Z"));
    expect((await db.query<{ removed: boolean }>("SELECT removed_at IS NOT NULL AS removed FROM repos WHERE id = 9002"))[0].removed).toBe(false);
  });

  it("m4: a suspended installation's jobs and runs leave the live queries", async () => {
    await link();
    await send("workflow_run", runPayload("in_progress"));
    await send("workflow_job", jobPayload("in_progress"));
    expect(await runningJobs(TEAM, db)).toHaveLength(1);
    await send("installation", inst("suspend", new Date().toISOString(), { suspended_at: new Date().toISOString() }));
    expect(await runningJobs(TEAM, db)).toEqual([]);
    expect(await recentRuns(TEAM, null, { db })).toEqual([]);
    expect(await jobById(TEAM, 1234, db)).toBeNull();
    expect((await teamInstallations(TEAM, db))[0].suspended).toBe(true);
  });

  it("m9: installation_target renamed updates the account and repo names", async () => {
    await link();
    await send("installation_target", { action: "renamed", installation: { id: 77 }, account: { id: 500, login: "acme-inc", type: "Organization" }, changes: { login: { from: "acme" } }, target_type: "Organization" });
    expect((await db.query("SELECT account_login FROM installations WHERE id = 77"))[0]).toEqual({ account_login: "acme-inc" });
    expect((await db.query("SELECT full_name FROM repos WHERE id = $1", [REPO.id]))[0]).toEqual({ full_name: "acme-inc/api" });
  });

  it("m7: prunes runs and jobs not updated for 30 days, and deliveries after 7", async () => {
    await send("workflow_run", runPayload("completed"));
    await send("workflow_job", jobPayload("completed"));
    await send("workflow_job", jobPayload("completed", { id: 2 }));
    await db.query("UPDATE workflow_jobs SET row_updated_at = now() - interval '31 days' WHERE id = 1234");
    await db.query("UPDATE workflow_runs SET row_updated_at = now() - interval '31 days'");
    await db.query("UPDATE deliveries SET received_at = now() - interval '8 days'");
    expect(await pruneOld(db)).toEqual({ deliveries: 3, runs: 1, jobs: 1 });
    expect((await db.query("SELECT id FROM workflow_jobs")).map((r) => r.id)).toEqual([2]);
  });

  it("M1: Disconnect unlinks only from the team that has it", async () => {
    await link(77, TEAM);
    expect(await disconnectInstallation(db, "org_other", 77)).toBe(false);
    expect((await db.query("SELECT team_id FROM installations WHERE id = 77"))[0]).toEqual({ team_id: "org_acme" });
    expect(await disconnectInstallation(db, "org_acme", 77)).toBe(true);
    expect((await db.query("SELECT team_id FROM installations WHERE id = 77"))[0]).toEqual({ team_id: null });
    // Free again: another team may now link it (after the GitHub checks).
    expect(await link(77, OTHER)).toBe("linked");
  });

  it("M3: subscribers of a team share one poller, which stops with the last", async () => {
    await link();
    lazyReconcileTeam.mockClear();
    const a: string[] = [];
    const b: string[] = [];
    const unA = subscribeTeamJobs(TEAM, (e) => a.push(e.job.id), { db, intervalMs: 250 });
    const unB = subscribeTeamJobs(TEAM, (e) => b.push(e.job.id), { db, intervalMs: 250 });
    await new Promise((r) => setTimeout(r, 50));
    expect(activePollers(db)).toBe(1);
    await new Promise((r) => setTimeout(r, 300));
    await send("workflow_job", jobPayload("in_progress", { id: 31 }));
    await new Promise((r) => setTimeout(r, 600));
    expect(a).toEqual(["31"]);
    expect(b).toEqual(["31"]);
    // The default lazy-reconcile hook ran once for the team's one poller.
    expect(lazyReconcileTeam).toHaveBeenCalledTimes(1);
    expect(lazyReconcileTeam).toHaveBeenCalledWith("org_acme", db);
    unA();
    await new Promise((r) => setTimeout(r, 10));
    expect(activePollers(db)).toBe(1);
    unB();
    await new Promise((r) => setTimeout(r, 10));
    expect(activePollers(db)).toBe(0);
  });

  it("M3: backs off from 2 s to 5 s to 10 s while nothing changes", () => {
    const p = { base: 2000, max: 10000 };
    expect([0, 2, 3, 9, 10, 50].map((idle) => nextDelay({ ...p, idle }))).toEqual([2000, 2000, 5000, 5000, 10000, 10000]);
  });

  it("M3: one poll query carries the database clock, even when nothing changed", async () => {
    const seen: string[] = [];
    const counting: Db = { ...db, query: (t, p) => (seen.push(t), db.query(t, p)) };
    const un = subscribeTeamJobs(TEAM, () => {}, { db: counting, intervalMs: 250 });
    await new Promise((r) => setTimeout(r, 600));
    un();
    expect(seen.length).toBeGreaterThanOrEqual(2);
    expect(seen.every((t) => t.includes("SELECT now() AS db_now"))).toBe(true);
  });
});

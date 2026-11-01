import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { migratedDb, truncateAll } from "@/lib/db/testing";
import type { Db } from "@/lib/db/types";
import { delivery, installationPayload, jobPayload, REPO, runPayload, SECRET } from "./fixtures";
import { jobById, jobEventFromPayload, onTeamJobChange, queuedJobs, recentRuns, runningJobs } from "./live";
import { linkInstallation } from "./store";
import { teamIdOf, type TeamId } from "./team";
import { handleWebhook } from "./webhook";

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
    const bad = jobPayload("queued") as unknown as { workflow_job: { name: unknown } };
    bad.workflow_job.name = null; // NOT NULL column: the insert fails
    await expect(send("workflow_job", bad, id)).rejects.toThrow();
    expect(await db.query("SELECT * FROM deliveries")).toEqual([]);
    expect((await send("workflow_job", jobPayload("queued"), id)).body).toEqual({ ok: true });
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

import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { migratedDb, truncateAll } from "@/lib/db/testing";
import type { Db } from "@/lib/db/types";
import type { JobRead, RateInfo } from "./api";
import { rateInfo } from "./api";
import { delivery, jobPayload, REPO, SECRET } from "./fixtures";
import { jobById, runningJobs } from "./live";
import { backoffUntil, LOW_REMAINING, pollTeamSteps, type StepPollGitHub } from "./step-poll";
import { linkInstallation } from "./store";
import { teamIdOf } from "./team";
import { handleWebhook } from "./webhook";

const TEAM = teamIdOf({ team: { orgId: "org_acme" } });
const RATE: RateInfo = { remaining: 4000, resetAt: null, retryAfterMs: null };

let db: Db;
beforeAll(async () => {
  db = await migratedDb();
});
afterAll(async () => db.close());
beforeEach(async () => {
  await truncateAll(db);
  await db.tx((q) => linkInstallation(q, { installation: { id: 77, account: { id: 500, login: "acme", type: "Organization" } }, teamId: TEAM, userId: "user_1", githubUserId: 1, repos: [REPO] }));
});

const send = (payload: unknown) => handleWebhook({ db, secret: SECRET, ...delivery("workflow_job", payload) });

/** The job as GitHub's REST API would return it now: a third step has started. */
function restJob(id: number) {
  const job = jobPayload("in_progress", { id }).workflow_job;
  const later = new Date(new Date(job.started_at as string).getTime() + 2 * 60_000).toISOString();
  return {
    ...job,
    steps: [
      { ...job.steps[0] },
      { ...job.steps[1], status: "completed", conclusion: "success", completed_at: later },
      { name: "Lint", status: "in_progress", conclusion: null, number: 3, started_at: later, completed_at: null },
    ],
  };
}

function fakeGitHub(answer: (jobId: number) => JobRead = (id) => ({ kind: "ok", job: restJob(id), rate: RATE })): StepPollGitHub & { calls: number[] } {
  const calls: number[] = [];
  return {
    calls,
    getJob: vi.fn(async (_inst: number, fullName: string, jobId: number) => {
      expect(fullName).toBe("acme/api");
      calls.push(jobId);
      return answer(jobId);
    }),
  };
}

describe("live step progress", () => {
  it("reads a running job's steps from GitHub and bumps its version, so the stream sends the update", async () => {
    await send(jobPayload("in_progress", { id: 1 }));
    const before = await jobById(TEAM, 1, db);
    expect(before?.currentStep).toMatchObject({ name: "Run tests" });
    const [{ v: v0 }] = await db.query<{ v: number }>("SELECT version AS v FROM workflow_jobs WHERE id = 1");

    const gh = fakeGitHub();
    expect(await pollTeamSteps(db, gh, TEAM)).toMatchObject({ polled: 1, updated: 1 });
    const after = await jobById(TEAM, 1, db);
    expect(after).toMatchObject({ status: "in_progress", currentStep: { index: 3, name: "Lint" }, stepsTotal: 3 });
    const [{ v: v1 }] = await db.query<{ v: number }>("SELECT version AS v FROM workflow_jobs WHERE id = 1");
    expect(Number(v1)).toBeGreaterThan(Number(v0));
  });

  it("reads each job at most once per interval (across instances), and never queued or finished jobs", async () => {
    await send(jobPayload("in_progress", { id: 1 }));
    await send(jobPayload("queued", { id: 2 }));
    await send(jobPayload("completed", { id: 3 }));
    const gh = fakeGitHub();
    await Promise.all([pollTeamSteps(db, gh, TEAM), pollTeamSteps(db, gh, TEAM)]); // two instances at once
    expect(gh.calls).toEqual([1]);
    await pollTeamSteps(db, gh, TEAM);
    expect(gh.calls).toEqual([1]); // within 15 s
    await pollTeamSteps(db, gh, TEAM, { intervalSec: 0 });
    expect(gh.calls).toEqual([1, 1]);
  });

  it("reads at most maxJobs per pass, least recently read first", async () => {
    for (const id of [1, 2, 3]) await send(jobPayload("in_progress", { id }));
    const gh = fakeGitHub();
    await pollTeamSteps(db, gh, TEAM, { maxJobs: 2 });
    expect(gh.calls).toHaveLength(2);
    await pollTeamSteps(db, gh, TEAM, { maxJobs: 2 });
    expect(gh.calls).toHaveLength(3);
    expect(new Set(gh.calls)).toEqual(new Set([1, 2, 3]));
  });

  it("backs the installation off on 403/429 until GitHub's reset, and skips it meanwhile", async () => {
    for (const id of [1, 2]) await send(jobPayload("in_progress", { id }));
    const gh = fakeGitHub(() => ({ kind: "limited", rate: { remaining: 0, resetAt: Date.now() + 120_000, retryAfterMs: null } }));
    expect(await pollTeamSteps(db, gh, TEAM)).toMatchObject({ limited: 1, polled: 1 });
    expect(gh.calls).toHaveLength(1); // the rest of the pass skips that installation
    const ok = fakeGitHub();
    await pollTeamSteps(db, ok, TEAM, { intervalSec: 0 });
    expect(ok.calls).toEqual([]);
    await db.query("UPDATE reconcile_state SET last_run_at = now() - interval '1 second' WHERE key = 'gh-backoff:77'");
    await pollTeamSteps(db, ok, TEAM, { intervalSec: 0 });
    expect(ok.calls).toHaveLength(2);
  });

  it("stops early when X-RateLimit-Remaining runs low", async () => {
    for (const id of [1, 2]) await send(jobPayload("in_progress", { id }));
    const gh = fakeGitHub((id) => ({ kind: "ok", job: restJob(id), rate: { remaining: LOW_REMAINING - 1, resetAt: Date.now() + 60_000, retryAfterMs: null } }));
    expect(await pollTeamSteps(db, gh, TEAM)).toMatchObject({ polled: 1, updated: 1 });
    const [row] = await db.query("SELECT 1 FROM reconcile_state WHERE key = 'gh-backoff:77' AND last_run_at > now()");
    expect(row).toBeTruthy();
  });

  it("a job GitHub no longer has, or an error, changes nothing", async () => {
    await send(jobPayload("in_progress", { id: 1 }));
    const before = await jobById(TEAM, 1, db);
    expect(await pollTeamSteps(db, fakeGitHub(() => ({ kind: "gone", rate: RATE })), TEAM)).toMatchObject({ gone: 1, updated: 0 });
    expect(await pollTeamSteps(db, fakeGitHub(() => ({ kind: "error", status: 500, rate: RATE })), TEAM, { intervalSec: 0 })).toMatchObject({ errors: 1 });
    expect(await jobById(TEAM, 1, db)).toEqual(before);
  });

  it("only polls its own team's jobs", async () => {
    await send(jobPayload("in_progress", { id: 1 }));
    const gh = fakeGitHub();
    await pollTeamSteps(db, gh, teamIdOf({ team: { orgId: "org_other" } }));
    expect(gh.calls).toEqual([]);
    expect((await runningJobs(TEAM, db)).length).toBe(1);
  });
});

describe("rate-limit headers", () => {
  it("reads remaining, reset and Retry-After", () => {
    const h = new Headers({ "x-ratelimit-remaining": "12", "x-ratelimit-reset": "1793570000", "retry-after": "30" });
    expect(rateInfo(h)).toEqual({ remaining: 12, resetAt: 1793570000_000, retryAfterMs: 30_000 });
    expect(rateInfo(new Headers())).toEqual({ remaining: null, resetAt: null, retryAfterMs: null });
  });

  it("backs off to Retry-After, else the reset time, else a minute; never more than an hour", () => {
    const now = 1_000_000;
    expect(backoffUntil({ remaining: null, resetAt: null, retryAfterMs: 30_000 }, now)).toBe(now + 30_000);
    expect(backoffUntil({ remaining: 0, resetAt: now + 90_000, retryAfterMs: null }, now)).toBe(now + 90_000);
    expect(backoffUntil({ remaining: null, resetAt: null, retryAfterMs: null }, now)).toBe(now + 60_000);
    expect(backoffUntil({ remaining: 0, resetAt: now + 5 * 3600_000, retryAfterMs: null }, now)).toBe(now + 3600_000);
  });
});

import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { migratedDb, truncateAll } from "@/lib/db/testing";
import type { Db } from "@/lib/db/types";
import { delivery, jobPayload, REPO, runPayload, SECRET } from "./fixtures";
import { getTeamRun, ghOutcome, listTeamRuns, parseRunQuery, teamHasRuns, teamJobMinutes, teamRunners } from "./pages";
import { linkInstallation } from "./store";
import { teamIdOf } from "./team";
import { handleWebhook } from "./webhook";

vi.mock("./lazy-reconcile", () => ({ lazyReconcileTeam: vi.fn(async () => undefined) }));

const TEAM = teamIdOf({ team: { orgId: "org_acme" } });
const OTHER = teamIdOf({ team: { orgId: "org_other" } });
const WEB = { ...REPO, id: 9002, name: "web", full_name: "acme/web" };
let db: Db;

beforeAll(async () => {
  db = await migratedDb();
});
afterAll(async () => db.close());
beforeEach(async () => {
  await truncateAll(db);
  await db.tx((q) => linkInstallation(q, { installation: { id: 77, account: { id: 500, login: "acme", type: "Organization" } }, teamId: TEAM, userId: "u", githubUserId: 1, repos: [REPO, WEB] }));
});

const send = (event: string, payload: unknown) => handleWebhook({ db, secret: SECRET, ...delivery(event, payload) });

async function run(id: number, status: "in_progress" | "completed", o: { repo?: typeof REPO; started?: string; conclusion?: string; branch?: string } = {}) {
  const p = runPayload(status, { id, number: id });
  const started = o.started ?? "2026-11-01T10:00:00Z";
  Object.assign(p.workflow_run, { run_started_at: started, created_at: started, updated_at: started, ...(o.conclusion ? { conclusion: o.conclusion } : {}), ...(o.branch ? { head_branch: o.branch } : {}) });
  await send("workflow_run", { ...p, repository: o.repo ?? REPO });
}

describe("runs list", () => {
  it("is empty for a linked team with no runs, and for another team", async () => {
    expect(await listTeamRuns(TEAM, db)).toEqual({ runs: [], hasMore: false, page: 1 });
    expect(await teamHasRuns(TEAM, db)).toBe(false);
    await run(1, "completed");
    expect(await teamHasRuns(TEAM, db)).toBe(true);
    expect((await listTeamRuns(OTHER, db)).runs).toEqual([]);
  });

  it("lists newest first with repo, workflow, number, branch, actor, outcome and duration", async () => {
    await run(1, "completed", { started: "2026-11-01T09:00:00Z" });
    await send("workflow_job", jobPayload("completed", { id: 11, runId: 1, t: "2026-11-01T09:00:00Z" })); // finishes at +4 min
    await run(2, "in_progress", { started: "2026-11-01T10:00:00Z", repo: WEB });
    const { runs } = await listTeamRuns(TEAM, db);
    expect(runs.map((r) => [r.id, r.repoFullName, r.outcome])).toEqual([
      [2, "acme/web", "running"],
      [1, "acme/api", "passed"],
    ]);
    expect(runs[1]).toMatchObject({ workflow: "CI", title: "Fix the flaky test", runNumber: 1, branch: "main", actor: "jamie", durationSeconds: 240 });
    expect(runs[0].durationSeconds).toBeNull();
  });

  it("filters by repo, branch and status, and pages", async () => {
    await run(1, "completed", { started: "2026-11-01T09:00:00Z" });
    await run(2, "completed", { started: "2026-11-01T09:10:00Z", conclusion: "failure", branch: "fix" });
    await run(3, "in_progress", { started: "2026-11-01T09:20:00Z", repo: WEB });
    expect((await listTeamRuns(TEAM, db, { repoId: WEB.id })).runs.map((r) => r.id)).toEqual([3]);
    expect((await listTeamRuns(TEAM, db, { branch: "fix" })).runs.map((r) => r.id)).toEqual([2]);
    expect((await listTeamRuns(TEAM, db, { status: "failed" })).runs.map((r) => r.id)).toEqual([2]);
    expect((await listTeamRuns(TEAM, db, { status: "running" })).runs.map((r) => r.id)).toEqual([3]);
    expect((await listTeamRuns(TEAM, db, { status: "passed" })).runs.map((r) => r.id)).toEqual([1]);
    const p1 = await listTeamRuns(TEAM, db, { perPage: 2 });
    expect([p1.runs.map((r) => r.id), p1.hasMore]).toEqual([[3, 2], true]);
    const p2 = await listTeamRuns(TEAM, db, { perPage: 2, page: 2 });
    expect([p2.runs.map((r) => r.id), p2.hasMore]).toEqual([[1], false]);
  });

  it("reads filters from the URL, ignoring what it does not know", () => {
    const repos = [{ id: 9001, slug: "api" }];
    expect(parseRunQuery({ repo: "api", branch: " main ", status: "failed", page: "2" }, repos)).toEqual({ repoSlug: "api", repoId: 9001, branch: "main", status: "failed", page: 2 });
    expect(parseRunQuery({ repo: "nope", status: "weird", page: "-1" }, repos)).toEqual({ repoSlug: null, repoId: null, branch: null, status: null, page: 1 });
  });

  it("maps GitHub's conclusions to the dashboard's words", () => {
    expect(["success", "failure", "timed_out", "cancelled", "skipped", "neutral"].map((c) => ghOutcome("completed", c))).toEqual(["passed", "failed", "failed", "cancelled", "skipped", "skipped"]);
    expect([ghOutcome("in_progress", null), ghOutcome("queued", null), ghOutcome("waiting", null)]).toEqual(["running", "queued", "queued"]);
  });
});

describe("a GitHub run", () => {
  it("has its jobs with status, duration, runner and steps", async () => {
    await run(5, "in_progress");
    await send("workflow_job", jobPayload("completed", { id: 51, runId: 5, t: "2026-11-01T10:00:00Z" }));
    await send("workflow_job", jobPayload("in_progress", { id: 52, runId: 5, t: "2026-11-01T10:05:00Z" }));
    const found = await getTeamRun(TEAM, db, 5);
    expect(found?.run).toMatchObject({ id: 5, repoFullName: "acme/api", outcome: "running" });
    expect(found?.jobs.map((j) => [j.id, j.outcome, j.durationSeconds, j.runnerName])).toEqual([
      [51, "passed", 180, "GitHub Actions 2"],
      [52, "running", null, "GitHub Actions 2"],
    ]);
    expect(found?.jobs[1].steps.map((s) => [s.number, s.name, s.outcome])).toEqual([
      [1, "Set up job", "passed"],
      [2, "Run tests", "running"],
    ]);
    expect(found?.jobs[0].labels).toEqual(["ubuntu-latest"]);
  });

  it("is not visible to another team, and a bad id is null", async () => {
    await run(5, "completed");
    expect(await getTeamRun(OTHER, db, 5)).toBeNull();
    expect(await getTeamRun(TEAM, db, 0)).toBeNull();
    expect(await getTeamRun(TEAM, db, 6)).toBeNull();
  });
});

describe("runners seen in CI", () => {
  it("groups GitHub-hosted runners by image and self-hosted ones by name, with jobs in 24 h and busy now", async () => {
    const now = new Date();
    const t = new Date(now.getTime() - 60 * 60_000).toISOString();
    await send("workflow_job", jobPayload("completed", { id: 1, t }));
    await send("workflow_job", jobPayload("in_progress", { id: 2, t }));
    const selfHosted = jobPayload("completed", { id: 3, t });
    Object.assign(selfHosted.workflow_job, { runner_name: "build-box-1", runner_group_name: "Default", labels: ["self-hosted", "linux"] });
    await send("workflow_job", selfHosted);
    const old = jobPayload("completed", { id: 4, t: new Date(now.getTime() - 30 * 3600_000).toISOString() });
    Object.assign(old.workflow_job, { runner_name: "GitHub Actions 9" });
    await send("workflow_job", old);
    const seen = await teamRunners(TEAM, db, { now });
    expect(seen.map((r) => [r.name, r.kind, r.jobs24h, r.busy])).toEqual([
      ["ubuntu-latest", "github-hosted", 2, 1],
      ["build-box-1", "self-hosted", 1, 0],
    ]);
    expect(seen[1]).toMatchObject({ group: "Default", labels: ["self-hosted", "linux"] });
    expect(await teamRunners(OTHER, db, { now })).toEqual([]);
  });
});

describe("CI minutes", () => {
  it("sums finished jobs' time per UTC day, with empty days as zero", async () => {
    const now = new Date();
    const t = new Date(now.getTime() - 2 * 3600_000).toISOString();
    await send("workflow_job", jobPayload("completed", { id: 1, t })); // 3 minutes (started +1, completed +4)
    await send("workflow_job", jobPayload("completed", { id: 2, t }));
    await send("workflow_job", jobPayload("in_progress", { id: 3, t })); // not finished: not counted
    const m = await teamJobMinutes(TEAM, db, { now, days: 7 });
    expect(m.days).toHaveLength(7);
    expect(m.totalMinutes).toBe(6);
    expect(m.jobs).toBe(2);
    expect(m.days.at(-1)?.day).toBe(now.toISOString().slice(0, 10));
    expect((await teamJobMinutes(OTHER, db, { now })).totalMinutes).toBe(0);
  });
});

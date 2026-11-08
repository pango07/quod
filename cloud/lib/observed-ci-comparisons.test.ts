import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { migratedDb, truncateAll } from "./db/testing";
import type { Db } from "./db/types";
import { CI_COMPARISON } from "./ci-comparisons";
import { delivery, jobPayload, REPO, runPayload, SECRET } from "./github/fixtures";
import { linkInstallation } from "./github/store";
import { teamIdOf } from "./github/team";
import { handleWebhook } from "./github/webhook";
import { getTeamRun } from "./github/pages";

vi.mock("server-only", () => ({}));
vi.mock("next/navigation", () => ({ unstable_rethrow: () => undefined }));
vi.mock("./github/lazy-reconcile", () => ({ lazyReconcileTeam: vi.fn(async () => undefined) }));
const source = vi.hoisted(() => ({ github: vi.fn() }));
vi.mock("./github/team-data", () => ({ teamGitHub: source.github }));
import { observedCiComparisons, readCiComparisons } from "./observed-ci-comparisons";

const TEAM = teamIdOf({ team: { orgId: "org_dogfood" } });
const OTHER = teamIdOf({ team: { orgId: "org_other" } });
const QUOD = { ...REPO, id: CI_COMPARISON.repoId, name: "quod", full_name: CI_COMPARISON.repository, private: false };
const now = new Date("2026-11-08T11:00:00Z");
let db: Db;
beforeAll(async () => { db = await migratedDb(); });
afterAll(async () => db.close());
beforeEach(async () => {
  vi.resetAllMocks();
  await truncateAll(db);
  await db.tx(q => linkInstallation(q, { installation: { id: 77, account: REPO.owner }, teamId: TEAM, userId: "u", githubUserId: 1, repos: [QUOD] }));
});
const send = (event: string, payload: unknown) => handleWebhook({ db, secret: SECRET, ...delivery(event, payload) });

async function cohort(id = 100, attempt = 1, baseline = true) {
  const p = runPayload("completed", { id, attempt, updated: "2026-11-08T10:06:00Z" });
  Object.assign(p.workflow_run, {
    name: CI_COMPARISON.workflow, event: "workflow_dispatch", run_started_at: "2026-11-08T10:00:00Z", created_at: "2026-11-08T10:00:00Z",
  });
  await send("workflow_run", { ...p, repository: QUOD });
  for (const [i, name] of [CI_COMPARISON.candidate, CI_COMPARISON.baseline].entries()) {
    if (i && !baseline) continue;
    const j = jobPayload("completed", { id: id * 10 + attempt * 2 + i, runId: id, attempt, repo: QUOD, t: "2026-11-08T10:00:00Z" });
    Object.assign(j.workflow_job, { name, workflow_name: CI_COMPARISON.workflow, completed_at: i ? "2026-11-08T10:05:00Z" : "2026-11-08T10:04:00Z" });
    await send("workflow_job", j);
  }
}

describe("team-scoped observed comparison read model", () => {
  it("reads full jobs and public evidence through the linked installation, without exposing another team's data", async () => {
    await cohort();
    expect(await readCiComparisons(TEAM, db, "7d", now)).toMatchObject({ pairs: [{ runId: 100, attempt: 1, candidate: { seconds: 180 }, baseline: { seconds: 240 }, savedSeconds: 60, cost: "public-standard-free" }] });
    expect((await readCiComparisons(OTHER, db, "7d", now)).pairs).toEqual([]);
    expect((await getTeamRun(TEAM, db, 100))?.run.repositoryPrivate).toBe(false);
    await db.query("UPDATE installations SET suspended_at = now() WHERE id = $1", [77]);
    expect((await readCiComparisons(TEAM, db, "7d", now)).pairs).toEqual([]);
  });

  it("uses only the run's latest attempt; an old successful baseline cannot fill a partial rerun", async () => {
    await cohort(100, 1);
    await cohort(100, 2, false);
    const detail = await getTeamRun(TEAM, db, 100);
    expect(detail?.run.runAttempt).toBe(2);
    expect(detail?.jobs.map(job => job.source?.runAttempt)).toEqual([2]);
    expect((await readCiComparisons(TEAM, db, "7d", now)).pairs).toEqual([]);
    await cohort(100, 2);
    expect((await readCiComparisons(TEAM, db, "7d", now)).pairs.map(pair => pair.attempt)).toEqual([2]);
  });

  it("rejects a failed job, wrong source, extra job and observations outside the selected range", async () => {
    await cohort();
    await db.query("UPDATE workflow_jobs SET conclusion = 'failure' WHERE name = $1", [CI_COMPARISON.baseline]);
    expect((await readCiComparisons(TEAM, db, "7d", now)).pairs).toEqual([]);
    await db.query("UPDATE workflow_jobs SET conclusion = 'success', head_sha = $1 WHERE name = $2", ["b".repeat(40), CI_COMPARISON.baseline]);
    expect((await readCiComparisons(TEAM, db, "7d", now)).pairs).toEqual([]);
    await db.query("UPDATE workflow_jobs SET head_sha = $1", ["a".repeat(40)]);
    const extra = jobPayload("completed", { id: 999, runId: 100, repo: QUOD, t: "2026-11-08T10:00:00Z" });
    await send("workflow_job", extra);
    expect((await readCiComparisons(TEAM, db, "7d", now)).pairs).toEqual([]);
    await db.query("DELETE FROM workflow_jobs WHERE id = $1", [999]);
    expect((await readCiComparisons(TEAM, db, "7d", new Date("2026-12-08T11:00:00Z"))).pairs).toEqual([]);
  });

  it("bounds detail lookups and preserves the latest-first order", async () => {
    for (let id = 100; id < 113; id++) await cohort(id);
    const result = await readCiComparisons(TEAM, db, "7d", now);
    expect(result.inspectedRuns).toBe(12);
    expect(result.pairs.map(pair => pair.runId)).toEqual([112, 111, 110, 109, 108, 107, 106, 105, 104, 103, 102, 101]);
  });

  it("shows the panel only for the authorized source, never a sample org, bare repo name or unrelated org", async () => {
    source.github.mockResolvedValue({ sample: true, source: null });
    expect((await observedCiComparisons({ orgId: "sample" }, "7d")).visible).toBe(false);
    source.github.mockResolvedValue({ sample: false, source: { teamId: OTHER, db, repos: [] } });
    expect((await observedCiComparisons({ orgId: "org_other" }, "7d")).visible).toBe(false);
    source.github.mockResolvedValue({ sample: false, source: { teamId: TEAM, db, repos: [{ id: QUOD.id, fullName: QUOD.full_name, slug: "quod" }] } });
    expect(await observedCiComparisons({ orgId: "org_dogfood" }, "7d", "other/quod")).toMatchObject({ visible: false, pairs: [] });
    expect(await observedCiComparisons({ orgId: "org_dogfood" }, "7d")).toMatchObject({ visible: true, available: true, pairs: [] });
    expect(source.github).toHaveBeenLastCalledWith("org_dogfood");
  });
});

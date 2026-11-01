import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { migratedDb, testBackend, truncateAll } from "@/lib/db/testing";
import type { Db } from "@/lib/db/types";
import { delivery, jobPayload, REPO, SECRET } from "@/lib/github/fixtures";
import { jobById, queuedJobs, runningJobs } from "@/lib/github/live";
import { linkInstallation } from "@/lib/github/store";
import { teamIdOf } from "@/lib/github/team";
import { handleWebhook } from "@/lib/github/webhook";

vi.mock("@/lib/github/lazy-reconcile", () => ({ lazyReconcileTeam: vi.fn(async () => undefined) }));

// jsonb columns must hold arrays, whichever driver wrote them. PGlite parses a
// JSON-text parameter as given; postgres.js (production) JSON-encodes a jsonb
// parameter again, which once stored arrays as jsonb strings (fixed in
// a45df10). Run with TEST_DATABASE_URL set to check the production driver
// against a real Postgres (lib/db/testing.ts); CI should do both.

const TEAM = teamIdOf({ team: { orgId: "org_acme" } });
const OTHER_REPO = { ...REPO, id: 9002, name: "web", full_name: "acme/web" };
let db: Db;

beforeAll(async () => {
  db = await migratedDb();
});
afterAll(async () => db.close());
beforeEach(async () => truncateAll(db));

const link = (repos = [REPO, OTHER_REPO]) =>
  db.tx((q) => linkInstallation(q, { installation: { id: 77, account: { id: 500, login: "acme", type: "Organization" } }, teamId: TEAM, userId: "user_1", githubUserId: 1, repos }));

describe(`jsonb round trip (${testBackend()})`, () => {
  it("stores labels and steps as jsonb arrays, and reads them back", async () => {
    await link();
    expect((await handleWebhook({ db, secret: SECRET, ...delivery("workflow_job", jobPayload("in_progress", { id: 1 })) })).status).toBeLessThan(300);
    const [row] = await db.query<{ l: string; s: string; n: number }>("SELECT jsonb_typeof(labels) AS l, jsonb_typeof(steps) AS s, jsonb_array_length(steps) AS n FROM workflow_jobs WHERE id = 1");
    expect(row).toEqual({ l: "array", s: "array", n: 2 });
    expect(await jobById(TEAM, 1, db)).toMatchObject({ labels: ["ubuntu-latest"], runner: "ubuntu-latest", stepsTotal: 2, currentStep: { name: "Run tests" } });
  });

  it("passes repo-id arrays as jsonb arrays (linking keeps the listed repos, removes the rest)", async () => {
    await link();
    await link([REPO]); // acme/web left the installation
    const rows = await db.query<{ id: number; removed: boolean }>("SELECT id, removed_at IS NOT NULL AS removed FROM repos ORDER BY id");
    expect(rows.map((r) => [Number(r.id), r.removed])).toEqual([
      [9001, false],
      [9002, true],
    ]);
  });

  it("reads rows written before the fix, whose arrays are jsonb strings, without rewriting them", async () => {
    await link();
    await handleWebhook({ db, secret: SECRET, ...delivery("workflow_job", jobPayload("in_progress", { id: 1 })) });
    await handleWebhook({ db, secret: SECRET, ...delivery("workflow_job", jobPayload("queued", { id: 2 })) });
    // What postgres.js used to store: the arrays' JSON text as a jsonb string.
    await db.query("UPDATE workflow_jobs SET labels = to_jsonb(labels::text), steps = to_jsonb(steps::text)");
    const [legacy] = await db.query<{ l: string }>("SELECT jsonb_typeof(labels) AS l FROM workflow_jobs WHERE id = 1");
    expect(legacy.l).toBe("string");
    expect((await runningJobs(TEAM, db))[0]).toMatchObject({ id: "1", labels: ["ubuntu-latest"], stepsTotal: 2, currentStep: { name: "Run tests" } });
    expect((await queuedJobs(TEAM, db))[0]).toMatchObject({ id: "2", runner: "ubuntu-latest" });
    const [still] = await db.query<{ l: string }>("SELECT jsonb_typeof(labels) AS l FROM workflow_jobs WHERE id = 1");
    expect(still.l).toBe("string"); // read only
  });
});

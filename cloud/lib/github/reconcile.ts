import type { Db } from "@/lib/db/types";
import type { HookDelivery } from "./api";
import { jobRowFromPayload, runRowFromPayload, type WorkflowJobPayload, type WorkflowRunPayload } from "./payloads";
import { ensureInstallation, markReconciled, pruneOld, upsertJob, upsertRun } from "./store";

// Healing lost webhooks (review M2). GitHub does not redeliver a failed
// delivery by itself, so a database blip during a webhook would lose that
// job transition for good. The reconcile cron (app/api/github/reconcile)
// runs this every 15 minutes:
//
// 1. Redeliver: list the App's deliveries of the last hour; for each
//    delivery GUID with no successful attempt, ask GitHub to send it again.
//    Needs no database: dedupe makes a redelivery that already landed a no-op.
// 2. Sweep: runs and jobs of linked repos still unfinished and not updated
//    (or checked) for `staleMinutes` are read back from GitHub's REST API
//    and applied through the same monotonic upserts as webhooks.
// 3. Retention: deliveries after 7 days, runs and jobs after 30.
//
// Steps 2 and 3 run only when the App had deliveries in the window (or
// `force`), so an idle Quod never wakes Neon from scale-to-zero.

export interface ReconcileGitHub {
  /** The App's deliveries since `sinceMs` (newest first). */
  listDeliveries(sinceMs: number): Promise<HookDelivery[]>;
  redeliver(deliveryId: number): Promise<boolean>;
  getRun(installationId: number, fullName: string, runId: number): Promise<Record<string, unknown> | null>;
  listRunJobs(installationId: number, fullName: string, runId: number): Promise<Record<string, unknown>[]>;
}

export interface ReconcileResult {
  deliveriesSeen: number;
  redelivered: number;
  swept: number;
  jobsHealed: number;
  pruned: { deliveries: number; runs: number; jobs: number } | null;
  errors: number;
}

const ok = (code: number) => code >= 200 && code < 400;

/**
 * The deliveries to ask again for: GUIDs with no successful attempt, the
 * newest failed attempt of each, at most `max`. A GUID whose redelivery
 * already succeeded is done.
 */
export function failedDeliveries(deliveries: HookDelivery[], max = 50): HookDelivery[] {
  const byGuid = new Map<string, HookDelivery[]>();
  for (const d of deliveries) byGuid.set(d.guid, [...(byGuid.get(d.guid) ?? []), d]);
  const out: HookDelivery[] = [];
  for (const attempts of byGuid.values()) {
    if (attempts.some((a) => ok(a.status_code))) continue;
    const latest = attempts.reduce((a, b) => (new Date(b.delivered_at) > new Date(a.delivered_at) ? b : a));
    out.push(latest);
  }
  return out.sort((a, b) => new Date(a.delivered_at).getTime() - new Date(b.delivered_at).getTime()).slice(0, max);
}

interface StaleRun {
  run_id: number;
  installation_id: number;
  repo_id: number;
  name: string;
  full_name: string;
  private: boolean;
}

/** Unfinished runs (and runs of unfinished jobs) in linked repos, longest unchecked first. */
export async function staleRuns(db: Db, staleMinutes: number, limit: number): Promise<StaleRun[]> {
  const rows = await db.query<StaleRun & { checked: Date }>(
    `SELECT t.run_id, t.installation_id, r.id AS repo_id, r.name, r.full_name, r.private, t.checked
     FROM (
       SELECT w.id AS run_id, w.installation_id, w.repo_id, COALESCE(w.reconciled_at, w.row_updated_at) AS checked
       FROM workflow_runs w
       WHERE w.status_rank < 3 AND COALESCE(w.reconciled_at, w.row_updated_at) < now() - make_interval(mins => $1)
       UNION
       SELECT j.run_id, j.installation_id, j.repo_id, COALESCE(w.reconciled_at, j.row_updated_at) AS checked
       FROM workflow_jobs j LEFT JOIN workflow_runs w ON w.id = j.run_id
       WHERE j.status_rank < 3 AND COALESCE(w.reconciled_at, j.row_updated_at) < now() - make_interval(mins => $1)
     ) t
     JOIN installations i ON i.id = t.installation_id AND i.team_id IS NOT NULL AND i.deleted_at IS NULL AND i.suspended_at IS NULL
     JOIN repos r ON r.id = t.repo_id AND r.installation_id = i.id AND r.removed_at IS NULL
     ORDER BY t.checked ASC
     LIMIT $2`,
    [staleMinutes, limit * 2],
  );
  const seen = new Set<number>();
  return rows.filter((r) => !seen.has(r.run_id) && seen.add(r.run_id)).slice(0, limit);
}

export async function reconcile(input: {
  db: () => Promise<Db>;
  gh: ReconcileGitHub;
  now?: Date;
  /** How far back to look for failed deliveries. Default 60 min (the cron runs every 15). */
  windowMs?: number;
  staleMinutes?: number;
  sweepLimit?: number;
  /** Sweep even when there were no deliveries in the window. */
  force?: boolean;
}): Promise<ReconcileResult> {
  const now = input.now ?? new Date();
  const result: ReconcileResult = { deliveriesSeen: 0, redelivered: 0, swept: 0, jobsHealed: 0, pruned: null, errors: 0 };

  const deliveries = await input.gh.listDeliveries(now.getTime() - (input.windowMs ?? 60 * 60_000));
  result.deliveriesSeen = deliveries.length;
  for (const d of failedDeliveries(deliveries)) {
    try {
      if (await input.gh.redeliver(d.id)) result.redelivered += 1;
    } catch {
      result.errors += 1;
    }
  }

  if (!deliveries.length && !input.force) return result;

  const db = await input.db();
  for (const run of await staleRuns(db, input.staleMinutes ?? 15, input.sweepLimit ?? 20)) {
    try {
      const repository = { id: run.repo_id, name: run.name, full_name: run.full_name, private: run.private };
      const installation = { id: run.installation_id };
      const ghRun = await input.gh.getRun(run.installation_id, run.full_name, run.run_id);
      const jobs = ghRun ? await input.gh.listRunJobs(run.installation_id, run.full_name, run.run_id) : [];
      await db.tx(async (q) => {
        await ensureInstallation(q, run.installation_id, null);
        if (ghRun) {
          const row = runRowFromPayload({ action: "reconcile", installation, repository, workflow_run: ghRun } as unknown as WorkflowRunPayload, now);
          if (row) await upsertRun(q, row);
        } else {
          // The run no longer exists on GitHub (deleted): its unfinished rows can never finish.
          await q.query("DELETE FROM workflow_jobs WHERE run_id = $1 AND status_rank < 3", [run.run_id]);
          await q.query("DELETE FROM workflow_runs WHERE id = $1 AND status_rank < 3", [run.run_id]);
        }
        for (const job of jobs) {
          const row = jobRowFromPayload({ action: "reconcile", installation, repository, workflow_job: job } as unknown as WorkflowJobPayload, now);
          if (row && (await upsertJob(q, row)) !== null) result.jobsHealed += 1;
        }
        await markReconciled(q, run.run_id);
      });
      result.swept += 1;
    } catch {
      result.errors += 1;
    }
  }
  result.pruned = await pruneOld(db);
  return result;
}

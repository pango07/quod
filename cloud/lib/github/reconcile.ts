import type { Db, Queryable } from "@/lib/db/types";
import type { HookDelivery } from "./api";
import { jobRowFromPayload, runRowFromPayload, type WorkflowJobPayload, type WorkflowRunPayload } from "./payloads";
import { ensureInstallation, markReconciled, pruneOld, upsertJob, upsertRun } from "./store";

// Healing lost webhooks (review M2). GitHub does not redeliver a failed
// delivery by itself, so a database blip during a webhook would lose that
// job transition for good. A pass has three parts:
//
// 1. Redeliver: list the App's failed deliveries in the window (App JWT,
//    GET /app/hook/deliveries?status=failure) and ask GitHub to send again
//    each GUID Quod never stored. Dedupe makes a duplicate harmless.
// 2. Sweep: runs and jobs of linked repos still unfinished and not updated
//    (or checked) for `staleMinutes` are read back from GitHub's REST API
//    and applied through the same monotonic upserts as webhooks.
// 3. Retention (daily pass only): deliveries after 7 days, runs and jobs after 30.
//
// Two triggers (Vercel Hobby allows one cron a day):
// - the daily cron (app/api/github/reconcile): every team, 25 h window, prune;
// - lazily while someone watches: a team's live poller calls
//   reconcileTeamIfDue at most every 15 minutes, throttled across
//   instances by reconcile_state (one atomic claim), in the background.
// Neither runs while nobody watches and the day's cron is done, so Neon can
// stay scaled to zero.

export const LAZY_INTERVAL_MINUTES = 15;

export interface ReconcileGitHub {
  /** The App's failed deliveries since `sinceMs` (newest first). */
  listFailedDeliveries(sinceMs: number): Promise<HookDelivery[]>;
  redeliver(deliveryId: number): Promise<boolean>;
  getRun(installationId: number, fullName: string, runId: number): Promise<Record<string, unknown> | null>;
  listRunJobs(installationId: number, fullName: string, runId: number): Promise<Record<string, unknown>[]>;
}

export interface ReconcileResult {
  failuresSeen: number;
  redelivered: number;
  swept: number;
  jobsHealed: number;
  pruned: { deliveries: number; runs: number; jobs: number } | null;
  errors: number;
}

const ok = (code: number) => code >= 200 && code < 400;

/**
 * The deliveries to ask again for: GUIDs with no successful attempt, the
 * newest failed attempt of each, at most `max`, oldest first.
 */
export function failedDeliveries(deliveries: HookDelivery[], max = 50): HookDelivery[] {
  const byGuid = new Map<string, HookDelivery[]>();
  for (const d of deliveries) byGuid.set(d.guid, [...(byGuid.get(d.guid) ?? []), d]);
  const out: HookDelivery[] = [];
  for (const attempts of byGuid.values()) {
    if (attempts.some((a) => ok(a.status_code))) continue;
    out.push(attempts.reduce((a, b) => (new Date(b.delivered_at) > new Date(a.delivered_at) ? b : a)));
  }
  return out.sort((a, b) => new Date(a.delivered_at).getTime() - new Date(b.delivered_at).getTime()).slice(0, max);
}

/**
 * The cross-instance throttle: true for exactly one caller per `minutes`
 * per key. One statement: insert the row, or move last_run_at only if it is
 * older than the interval; RETURNING says whether this caller won.
 */
export async function claimReconcile(q: Queryable, key: string, minutes: number): Promise<boolean> {
  const rows = await q.query(
    `INSERT INTO reconcile_state (key, last_run_at) VALUES ($1, now())
     ON CONFLICT (key) DO UPDATE SET last_run_at = now()
     WHERE reconcile_state.last_run_at < now() - make_interval(mins => $2)
     RETURNING key`,
    [key, minutes],
  );
  return rows.length === 1;
}

interface StaleRun {
  run_id: number;
  installation_id: number;
  repo_id: number;
  name: string;
  full_name: string;
  private: boolean;
}

/** Unfinished runs (and runs of unfinished jobs) in linked repos, longest unchecked first; one team's with `teamId`. */
export async function staleRuns(db: Queryable, staleMinutes: number, limit: number, teamId: string | null = null): Promise<StaleRun[]> {
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
       AND ($3::text IS NULL OR i.team_id = $3::text)
     JOIN repos r ON r.id = t.repo_id AND r.installation_id = i.id AND r.removed_at IS NULL
     ORDER BY t.checked ASC
     LIMIT $2`,
    [staleMinutes, limit * 2, teamId],
  );
  const seen = new Set<number>();
  return rows.filter((r) => !seen.has(r.run_id) && seen.add(r.run_id)).slice(0, limit);
}

/** GUIDs Quod already stored: those landed after all (dedupe would drop the redelivery anyway). */
async function storedGuids(db: Queryable, guids: string[]): Promise<Set<string>> {
  if (!guids.length) return new Set();
  const rows = await db.query<{ id: string }>("SELECT id FROM deliveries WHERE id IN (SELECT jsonb_array_elements_text($1::jsonb))", [JSON.stringify(guids)]);
  return new Set(rows.map((r) => r.id));
}

export async function reconcile(input: {
  db: Db;
  gh: ReconcileGitHub;
  now?: Date;
  /** Only this team's installations (the lazy pass). Without it, every linked installation (the daily pass). */
  teamId?: string | null;
  /** How far back to look for failed deliveries. */
  windowMs: number;
  staleMinutes?: number;
  sweepLimit?: number;
  /** Retention too (the daily pass). */
  prune?: boolean;
}): Promise<ReconcileResult> {
  const { db } = input;
  const now = input.now ?? new Date();
  const teamId = input.teamId ?? null;
  const result: ReconcileResult = { failuresSeen: 0, redelivered: 0, swept: 0, jobsHealed: 0, pruned: null, errors: 0 };

  // 1. Redeliver.
  let failures = failedDeliveries(await input.gh.listFailedDeliveries(now.getTime() - input.windowMs), 100);
  if (teamId) {
    const mine = new Set((await db.query<{ id: number }>("SELECT id FROM installations WHERE team_id = $1 AND deleted_at IS NULL", [teamId])).map((r) => Number(r.id)));
    failures = failures.filter((d) => d.installation_id != null && mine.has(Number(d.installation_id)));
  }
  const stored = await storedGuids(db, failures.map((d) => d.guid));
  failures = failures.filter((d) => !stored.has(d.guid)).slice(0, 50);
  result.failuresSeen = failures.length;
  for (const d of failures) {
    try {
      if (await input.gh.redeliver(d.id)) result.redelivered += 1;
    } catch {
      result.errors += 1;
    }
  }

  // 2. Sweep.
  for (const run of await staleRuns(db, input.staleMinutes ?? 15, input.sweepLimit ?? 20, teamId)) {
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

  // 3. Retention.
  if (input.prune) result.pruned = await pruneOld(db);
  return result;
}

/** The daily cron's pass: every team, the last 25 hours of failures, retention. */
export function reconcileDaily(db: Db, gh: ReconcileGitHub, now = new Date()): Promise<ReconcileResult> {
  return reconcile({ db, gh, now, windowMs: 25 * 60 * 60_000, sweepLimit: 100, prune: true });
}

/**
 * The lazy pass for one team, while someone watches it: at most once per
 * LAZY_INTERVAL_MINUTES across all instances (the reconcile_state claim);
 * null when another instance (or an earlier tick) already ran it.
 */
export async function reconcileTeamIfDue(db: Db, gh: ReconcileGitHub, teamId: string, now = new Date()): Promise<ReconcileResult | null> {
  if (!(await claimReconcile(db, `team:${teamId}`, LAZY_INTERVAL_MINUTES))) return null;
  // 3 h of failures: also covers a gap between viewing sessions; the daily pass covers the rest.
  return reconcile({ db, gh, now, teamId, windowMs: 3 * 60 * 60_000 });
}

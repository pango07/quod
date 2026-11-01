import type { Queryable } from "@/lib/db/types";
import type { JobRead, RateInfo } from "./api";
import { jobRowFromPayload, type WorkflowJobPayload } from "./payloads";
import { upsertJob } from "./store";

// Live step progress. GitHub's workflow_job webhooks fire when a job is
// queued, starts and completes, never per step, so "Step 2 of 8" would stay
// put for the whole job. While someone watches a team (its live poller runs,
// lib/github/live.ts), this reads the team's running jobs back from GitHub
// (GET /repos/{o}/{r}/actions/jobs/{id}, installation token) about every 15 s
// and applies them through the same monotonic upsert as webhooks: a newer
// step bumps the row's version and the stream sends a job.started update.
//
// Bounded:
// - at most `maxJobs` jobs per team per pass, least recently read first;
// - each job at most once per `intervalSec` across all instances: the claim
//   is one conditional UPDATE of workflow_jobs.steps_polled_at (SKIP LOCKED);
// - a 403/429, or X-RateLimit-Remaining under LOW_REMAINING, backs the
//   installation off (reconcile_state "gh-backoff:<id>") until the reset time
//   GitHub gives (Retry-After, X-RateLimit-Reset), else for a minute.
// Nothing runs when nobody watches.

export const STEP_POLL_INTERVAL_S = 15;
export const MAX_JOBS_PER_PASS = 10;
/** Leave this much of an installation's hourly REST budget to everything else. */
export const LOW_REMAINING = 200;
const DEFAULT_BACKOFF_MS = 60_000;
const MAX_BACKOFF_MS = 60 * 60_000;

export interface StepPollGitHub {
  getJob(installationId: number, fullName: string, jobId: number): Promise<JobRead>;
}

export interface StepPollResult {
  polled: number;
  updated: number;
  gone: number;
  limited: number;
  errors: number;
}

interface Claimed {
  id: number;
  installation_id: number;
  repo_id: number;
  name: string;
  full_name: string;
  private: boolean;
}

const backoffKey = (installationId: number) => `gh-backoff:${installationId}`;

/**
 * Claims up to `limit` of the team's running jobs not read in the last
 * `intervalSec` (and whose installation is not backed off), marking them
 * read now. Concurrent callers never get the same job.
 */
export async function claimRunningJobs(q: Queryable, teamId: string, opts: { intervalSec?: number; limit?: number } = {}): Promise<Claimed[]> {
  const rows = await q.query<Claimed>(
    `WITH c AS (
       SELECT j.id
       FROM workflow_jobs j
       JOIN installations i ON i.id = j.installation_id AND i.team_id = $1 AND i.deleted_at IS NULL AND i.suspended_at IS NULL
       JOIN repos r ON r.id = j.repo_id AND r.installation_id = i.id AND r.removed_at IS NULL
       WHERE j.status = 'in_progress'
         AND j.row_updated_at > now() - make_interval(hours => 24)
         AND (j.steps_polled_at IS NULL OR j.steps_polled_at < now() - make_interval(secs => $2::double precision))
         AND NOT EXISTS (SELECT 1 FROM reconcile_state s WHERE s.key = 'gh-backoff:' || j.installation_id::text AND s.last_run_at > now())
       ORDER BY j.steps_polled_at ASC NULLS FIRST, j.id
       LIMIT $3
       FOR UPDATE OF j SKIP LOCKED
     )
     UPDATE workflow_jobs j SET steps_polled_at = now()
     FROM c, repos r
     WHERE j.id = c.id AND r.id = j.repo_id
     RETURNING j.id, j.installation_id, j.repo_id, r.name, r.full_name, r.private`,
    [teamId, opts.intervalSec ?? STEP_POLL_INTERVAL_S, Math.max(1, Math.min(opts.limit ?? MAX_JOBS_PER_PASS, 100))],
  );
  return rows.map((r) => ({ ...r, id: Number(r.id), installation_id: Number(r.installation_id), repo_id: Number(r.repo_id), private: Boolean(r.private) }));
}

/** Backs an installation off until `untilMs` (never shortens a longer back-off already set). */
export async function backOff(q: Queryable, installationId: number, untilMs: number): Promise<void> {
  await q.query(
    `INSERT INTO reconcile_state (key, last_run_at) VALUES ($1, to_timestamp($2::double precision / 1000))
     ON CONFLICT (key) DO UPDATE SET last_run_at = GREATEST(reconcile_state.last_run_at, EXCLUDED.last_run_at)`,
    [backoffKey(installationId), untilMs],
  );
}

/** When to try again after GitHub limited us (or nearly did), bounded to an hour. */
export function backoffUntil(rate: RateInfo, nowMs: number): number {
  const until = rate.retryAfterMs !== null ? nowMs + rate.retryAfterMs : rate.resetAt !== null && rate.resetAt > nowMs ? rate.resetAt : nowMs + DEFAULT_BACKOFF_MS;
  return Math.min(Math.max(until, nowMs + 1000), nowMs + MAX_BACKOFF_MS);
}

/** One pass for one team: claim its running jobs, read each from GitHub, apply what changed. */
export async function pollTeamSteps(
  db: Queryable,
  gh: StepPollGitHub,
  teamId: string,
  opts: { now?: () => number; intervalSec?: number; maxJobs?: number } = {},
): Promise<StepPollResult> {
  const now = opts.now ?? Date.now;
  const result: StepPollResult = { polled: 0, updated: 0, gone: 0, limited: 0, errors: 0 };
  const claimed = await claimRunningJobs(db, teamId, { intervalSec: opts.intervalSec, limit: opts.maxJobs });
  const stopped = new Set<number>(); // installations backed off during this pass
  for (const job of claimed) {
    if (stopped.has(job.installation_id)) continue;
    let read: JobRead;
    try {
      read = await gh.getJob(job.installation_id, job.full_name, job.id);
    } catch {
      result.errors += 1;
      continue;
    }
    result.polled += 1;
    if (read.kind === "limited") {
      result.limited += 1;
      stopped.add(job.installation_id);
      await backOff(db, job.installation_id, backoffUntil(read.rate, now()));
      continue;
    }
    if (read.kind === "ok") {
      const row = jobRowFromPayload(
        {
          action: "steps",
          installation: { id: job.installation_id },
          repository: { id: job.repo_id, name: job.name, full_name: job.full_name, private: job.private },
          workflow_job: read.job,
        } as unknown as WorkflowJobPayload,
        new Date(now()),
      );
      // Only this job's own row: GitHub must answer with the job we asked for.
      if (row && row.id === job.id && (await upsertJob(db, row)) !== null) result.updated += 1;
    } else if (read.kind === "gone") {
      result.gone += 1; // the reconcile sweep removes jobs whose run is gone
    } else {
      result.errors += 1;
    }
    if (read.rate.remaining !== null && read.rate.remaining < LOW_REMAINING) {
      stopped.add(job.installation_id);
      await backOff(db, job.installation_id, backoffUntil({ ...read.rate, retryAfterMs: null }, now()));
    }
  }
  return result;
}

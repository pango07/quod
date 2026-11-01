import type { Db, Row } from "@/lib/db/types";
import { eventTypeFor, type LiveJob, type LiveJobEvent, type LiveJobStatus, type LiveRun } from "@/lib/live-events";
import type { GhStep, WorkflowJobPayload } from "./payloads";
import { jobRowFromPayload } from "./payloads";
import type { TeamId } from "./team";

// The query and event layer over the GitHub App's tables, for the SSE route
// and the "Now running" UI (branch cloud/live). Server-side only.
//
// Team scoping: every function takes a TeamId (lib/github/team.ts), which
// only comes from requireTeam's TeamContext. Rows are visible to a team only
// through an installation linked to it (installations.team_id), not deleted,
// and a repo still in that installation.
//
// `db` is optional everywhere: the app's shared database by default, PGlite in tests.

const resolve = async (db?: Db): Promise<Db> => db ?? (await import("@/lib/db")).getDb();

/** Queued or running jobs older than this are left out (a lost "completed" delivery must not pin a job forever). */
export const ACTIVE_WINDOW_HOURS = 24;

const JOB_SELECT = `
  SELECT j.id, j.run_id, j.run_attempt, j.workflow_name, j.name, j.head_branch, j.head_sha, j.sender_login,
         j.status, j.conclusion, j.labels, j.runner_name, j.html_url, j.created_at, j.started_at, j.completed_at,
         j.steps, j.version, j.row_updated_at,
         r.name AS repo_name, r.full_name AS repo_full_name,
         w.run_number, w.actor_login, w.name AS run_name
  FROM workflow_jobs j
  JOIN installations i ON i.id = j.installation_id AND i.team_id = $1 AND i.deleted_at IS NULL
  JOIN repos r ON r.id = j.repo_id AND r.installation_id = i.id AND r.removed_at IS NULL
  LEFT JOIN workflow_runs w ON w.id = j.run_id`;

// ---------------------------------------------------------------- Row and payload mapping

const iso = (v: unknown): string | undefined => (v instanceof Date ? v.toISOString() : typeof v === "string" && v ? new Date(v).toISOString() : undefined);
const str = (v: unknown, fallback = ""): string => (typeof v === "string" ? v : fallback);

function liveStatus(status: string): LiveJobStatus {
  return status === "completed" ? "completed" : status === "in_progress" ? "in_progress" : "queued";
}

function liveConclusion(c: string | null | undefined): LiveJob["conclusion"] {
  if (c === "success") return "success";
  if (c === "failure" || c === "timed_out" || c === "startup_failure") return "failure";
  if (c === "cancelled") return "cancelled";
  return undefined;
}

export function runnerLabel(labels: string[]): string {
  return labels.length === 1 ? labels[0] : labels.length ? `[${labels.join(", ")}]` : "";
}

function currentStep(steps: GhStep[], status: LiveJobStatus): LiveJob["currentStep"] {
  if (status !== "in_progress") return undefined;
  const s = steps.find((x) => x.status === "in_progress") ?? steps.find((x) => x.status !== "completed");
  return s ? { index: s.number, name: s.name } : undefined;
}

/** A stored job row (JOB_SELECT's columns) as a LiveJob. */
export function liveJobFromRow(row: Row): LiveJob {
  const status = liveStatus(str(row.status));
  const labels = Array.isArray(row.labels) ? (row.labels as unknown[]).map(String) : [];
  const steps = Array.isArray(row.steps) ? (row.steps as GhStep[]) : [];
  const conclusion = status === "completed" ? liveConclusion(row.conclusion as string | null) : undefined;
  const job: LiveJob = {
    id: String(row.id),
    repoSlug: str(row.repo_name),
    repoName: str(row.repo_full_name),
    runNumber: typeof row.run_number === "number" ? row.run_number : 0,
    workflow: str(row.workflow_name, str(row.run_name)),
    job: str(row.name),
    branch: str(row.head_branch),
    sha: str(row.head_sha),
    actor: str(row.actor_login, str(row.sender_login)),
    runner: runnerLabel(labels),
    githubUrl: str(row.html_url),
    status,
    queuedAt: iso(row.created_at) ?? iso(row.row_updated_at) ?? new Date(0).toISOString(),
    source: "github",
    runId: Number(row.run_id),
    runAttempt: Number(row.run_attempt ?? 1),
    labels,
  };
  // GitHub fills started_at on queued jobs too; only report it once the job runs.
  if (status !== "queued") job.startedAt = iso(row.started_at);
  if (status === "completed") job.finishedAt = iso(row.completed_at);
  if (conclusion) job.conclusion = conclusion;
  if (status === "completed" && typeof row.conclusion === "string") job.githubConclusion = row.conclusion;
  const step = currentStep(steps, status);
  if (step) job.currentStep = step;
  if (steps.length) job.stepsTotal = steps.length;
  if (typeof row.runner_name === "string" && row.runner_name) job.runnerName = row.runner_name;
  return job;
}

/** A stored job row as its event: job.queued, job.started or job.finished by status. */
export function jobEventFromRow(row: Row): LiveJobEvent {
  const job = liveJobFromRow(row);
  return { id: `gh-${Number(row.version)}`, at: iso(row.row_updated_at) ?? new Date().toISOString(), type: eventTypeFor(job.status), job };
}

/**
 * A workflow_job webhook payload as an event, before (or without) storing it.
 * The run number is 0 and the actor is the sender: a payload does not carry
 * them. Prefer jobEventFromRow on the stored row, which has both.
 */
export function jobEventFromPayload(p: WorkflowJobPayload, opts: { version: number; at?: Date }): LiveJobEvent | null {
  const at = opts.at ?? new Date();
  const row = jobRowFromPayload(p, at);
  if (!row) return null;
  return jobEventFromRow({
    ...row,
    repo_name: p.repository.name,
    repo_full_name: p.repository.full_name,
    run_number: null,
    actor_login: null,
    version: opts.version,
    row_updated_at: at,
  });
}

// ---------------------------------------------------------------- Queries

/** Jobs running now, newest first. */
export async function runningJobs(teamId: TeamId, db?: Db): Promise<LiveJob[]> {
  const q = await resolve(db);
  const rows = await q.query(`${JOB_SELECT} WHERE j.status = 'in_progress' AND j.event_at > now() - make_interval(hours => $2) ORDER BY j.started_at DESC NULLS LAST, j.id DESC LIMIT 200`, [teamId, ACTIVE_WINDOW_HOURS]);
  return rows.map(liveJobFromRow);
}

/** Jobs waiting for a runner (queued, waiting, pending), oldest first. */
export async function queuedJobs(teamId: TeamId, db?: Db): Promise<LiveJob[]> {
  const q = await resolve(db);
  const rows = await q.query(`${JOB_SELECT} WHERE j.status_rank = 1 AND j.event_at > now() - make_interval(hours => $2) ORDER BY j.created_at ASC NULLS LAST, j.id ASC LIMIT 200`, [teamId, ACTIVE_WINDOW_HOURS]);
  return rows.map(liveJobFromRow);
}

/** One job, if the team can see it. */
export async function jobById(teamId: TeamId, jobId: number | string, db?: Db): Promise<LiveJob | null> {
  const id = typeof jobId === "number" ? jobId : /^\d{1,15}$/.test(jobId) ? Number(jobId) : NaN;
  if (!Number.isSafeInteger(id) || id <= 0) return null;
  const q = await resolve(db);
  const [row] = await q.query(`${JOB_SELECT} WHERE j.id = $2`, [teamId, id]);
  return row ? liveJobFromRow(row) : null;
}

/**
 * Recent workflow runs, newest first. `repo` is "owner/name" or a bare repo
 * name (which may match one repo per owner the team has linked).
 */
export async function recentRuns(teamId: TeamId, repo?: string | null, opts: { limit?: number; db?: Db } = {}): Promise<LiveRun[]> {
  const q = await resolve(opts.db);
  const limit = Math.min(Math.max(Math.trunc(opts.limit ?? 50), 1), 200);
  const params: unknown[] = [teamId, limit];
  let where = "";
  if (repo) {
    if (repo.length > 200) return [];
    params.push(repo);
    where = repo.includes("/") ? "AND lower(r.full_name) = lower($3)" : "AND lower(r.name) = lower($3)";
  }
  const rows = await q.query(
    `SELECT w.*, r.name AS repo_name, r.full_name AS repo_full_name
     FROM workflow_runs w
     JOIN installations i ON i.id = w.installation_id AND i.team_id = $1 AND i.deleted_at IS NULL
     JOIN repos r ON r.id = w.repo_id AND r.installation_id = i.id AND r.removed_at IS NULL
     WHERE true ${where}
     ORDER BY COALESCE(w.run_started_at, w.created_at, w.gh_updated_at) DESC, w.id DESC
     LIMIT $2`,
    params,
  );
  return rows.map((row) => ({
    id: Number(row.id),
    repoSlug: str(row.repo_name),
    repoName: str(row.repo_full_name),
    runNumber: typeof row.run_number === "number" ? row.run_number : null,
    runAttempt: Number(row.run_attempt ?? 1),
    workflow: str(row.name),
    title: str(row.display_title, str(row.name)),
    branch: str(row.head_branch),
    sha: str(row.head_sha),
    actor: str(row.actor_login),
    event: str(row.event),
    status: liveStatus(str(row.status)),
    ...(row.status === "completed" && typeof row.conclusion === "string" ? { githubConclusion: row.conclusion } : {}),
    githubUrl: str(row.html_url),
    startedAt: iso(row.run_started_at) ?? null,
    updatedAt: iso(row.gh_updated_at) ?? new Date(0).toISOString(),
  }));
}

/** The installations linked to a team, with their repos (for the linking page and settings). */
export async function teamInstallations(teamId: TeamId, db?: Db): Promise<{ id: number; account: string; accountType: string; suspended: boolean; repos: { id: number; fullName: string; private: boolean }[] }[]> {
  const q = await resolve(db);
  const rows = await q.query(
    `SELECT i.id, i.account_login, i.account_type, i.suspended_at, r.id AS repo_id, r.full_name, r.private
     FROM installations i LEFT JOIN repos r ON r.installation_id = i.id AND r.removed_at IS NULL
     WHERE i.team_id = $1 AND i.deleted_at IS NULL
     ORDER BY i.id, r.full_name`,
    [teamId],
  );
  const out = new Map<number, { id: number; account: string; accountType: string; suspended: boolean; repos: { id: number; fullName: string; private: boolean }[] }>();
  for (const r of rows) {
    const id = Number(r.id);
    if (!out.has(id)) out.set(id, { id, account: str(r.account_login), accountType: str(r.account_type), suspended: r.suspended_at != null, repos: [] });
    if (r.repo_id != null) out.get(id)!.repos.push({ id: Number(r.repo_id), fullName: str(r.full_name), private: Boolean(r.private) });
  }
  return [...out.values()];
}

// ---------------------------------------------------------------- Change feed

export interface ChangeFeedOptions {
  signal: AbortSignal;
  /** Poll interval. Default 2000 ms. */
  intervalMs?: number;
  /** Resume after this event id ("gh-<version>", from Last-Event-ID). Without it, only changes from now on. */
  after?: string | null;
  /** How far back each poll looks, to catch writes that committed out of version order. Default 15 s. */
  lookbackMs?: number;
  db?: Db;
}

const sleep = (ms: number, signal: AbortSignal) =>
  new Promise<void>((done) => {
    if (signal.aborted) return done();
    const t = setTimeout(finish, ms);
    function finish() {
      clearTimeout(t);
      signal.removeEventListener("abort", finish);
      done();
    }
    signal.addEventListener("abort", finish, { once: true });
  });

/**
 * Job changes for a team, as events, until `signal` aborts. A short poll on
 * workflow_jobs.row_updated_at, not LISTEN/NOTIFY: Neon's pooled connections
 * (PgBouncer, transaction mode) cannot LISTEN, and a function holding a
 * direct connection open per viewer would use up Neon's connection limit.
 *
 * Each poll reads the rows changed in the last `lookbackMs` (by database
 * time) and yields those whose version it has not yielded yet, so a write
 * that commits after a later one is still seen. One query per poll per
 * subscriber: the SSE layer should share one feed per team per instance.
 */
export async function* onTeamJobChange(teamId: TeamId, opts: ChangeFeedOptions): AsyncGenerator<LiveJobEvent> {
  const { signal } = opts;
  const intervalMs = Math.max(250, opts.intervalMs ?? 2000);
  const lookbackMs = Math.max(intervalMs * 2, opts.lookbackMs ?? 15000);
  const q = await resolve(opts.db);
  const seen = new Map<number, { version: number; at: number }>();
  const afterMatch = typeof opts.after === "string" ? /^gh-(\d{1,15})$/.exec(opts.after) : null;
  const after = afterMatch ? Number(afterMatch[1]) : null;

  const changedSince = async (sinceMs: number) => {
    const rows = await q.query(`${JOB_SELECT} WHERE j.row_updated_at > to_timestamp($2::double precision / 1000) ORDER BY j.version ASC LIMIT 1000`, [teamId, sinceMs]);
    const [{ now }] = await q.query<{ now: Date }>("SELECT now() AS now");
    return { rows, now: now.getTime() };
  };

  // Start: remember what is already there, so only later changes are yielded.
  const [{ now: start }] = await q.query<{ now: Date }>("SELECT now() AS now");
  let dbNow = start.getTime();
  {
    const { rows } = await changedSince(dbNow - lookbackMs);
    for (const r of rows) seen.set(Number(r.id), { version: Number(r.version), at: dbNow });
  }
  // Resuming: everything after the client's last event id.
  if (after !== null && !signal.aborted) {
    const rows = await q.query(`${JOB_SELECT} WHERE j.version > $2 ORDER BY j.version ASC LIMIT 1000`, [teamId, after]);
    for (const r of rows) {
      seen.set(Number(r.id), { version: Number(r.version), at: dbNow });
      yield jobEventFromRow(r);
    }
  }

  while (!signal.aborted) {
    await sleep(intervalMs, signal);
    if (signal.aborted) break;
    const { rows, now } = await changedSince(dbNow - lookbackMs);
    dbNow = now;
    for (const r of rows) {
      const id = Number(r.id);
      const version = Number(r.version);
      const prev = seen.get(id);
      if (prev && prev.version >= version) continue;
      seen.set(id, { version, at: dbNow });
      yield jobEventFromRow(r);
      if (signal.aborted) return;
    }
    // Forget rows that left the lookback window long ago.
    for (const [id, s] of seen) if (s.at < dbNow - 4 * lookbackMs) seen.delete(id);
  }
}

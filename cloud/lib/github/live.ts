import type { Db, Row } from "@/lib/db/types";
import { eventTypeFor, versionOfEventId, type LiveJob, type LiveJobEvent, type LiveJobStatus, type LiveRun } from "@/lib/live-events";
import type { GhStep, WorkflowJobPayload } from "./payloads";
import { jobRowFromPayload } from "./payloads";
import type { TeamId } from "./team";

// The query and event layer over the GitHub App's tables, for the SSE route
// and the "Now running" UI (branch cloud/live). Server-side only.
//
// Team scoping: every function takes a TeamId (lib/github/team.ts), which
// only comes from requireTeam's TeamContext. Rows are visible to a team only
// through an installation linked to it (installations.team_id), not deleted or suspended,
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
  JOIN installations i ON i.id = j.installation_id AND i.team_id = $1 AND i.deleted_at IS NULL AND i.suspended_at IS NULL
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

/**
 * The team's job changes stored in the last `windowMs` (by database time), as
 * events, oldest version first. The live adapter replays these when a stream
 * starts, so a change that committed while its snapshot was being taken is
 * not lost; repeats are dropped by version downstream.
 */
export async function recentJobEvents(teamId: TeamId, windowMs: number, db?: Db): Promise<LiveJobEvent[]> {
  const q = await resolve(db);
  const rows = await q.query(`${JOB_SELECT} WHERE j.row_updated_at > now() - make_interval(secs => $2::double precision) ORDER BY j.version ASC LIMIT 1000`, [teamId, windowMs / 1000]);
  return rows.map(jobEventFromRow);
}

/** The highest job version the team can see now (0 if none): a snapshot's stream position. */
export async function maxJobVersion(teamId: TeamId, db?: Db): Promise<number> {
  const q = await resolve(db);
  const [row] = await q.query(`SELECT COALESCE(max(j.version), 0) AS v FROM (${JOB_SELECT}) j`, [teamId]);
  return Number(row?.v ?? 0);
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
     JOIN installations i ON i.id = w.installation_id AND i.team_id = $1 AND i.deleted_at IS NULL AND i.suspended_at IS NULL
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

// ---------------------------------------------------------------- Change feed: one shared poller per team

export interface PollOptions {
  /** Poll interval while jobs are changing. Default 2000 ms. */
  intervalMs?: number;
  /** Ceiling after idle polls back off (2 s, then 5 s, then this). Default 10000 ms. */
  maxIntervalMs?: number;
  /** How far back each poll looks, to catch writes that committed out of version order. Default 15 s. */
  lookbackMs?: number;
  db?: Db;
  /**
   * Called in the background while the poller runs (someone is watching), at
   * most every `reconcileEveryMs` per instance: lazy GitHub reconciliation.
   * Default: lib/github/lazy-reconcile.ts, whose database claim makes it at
   * most once per 15 minutes per team across instances. null turns it off.
   */
  reconcile?: ((teamId: TeamId, db: Db) => Promise<unknown>) | null;
  /** Default 15 minutes. */
  reconcileEveryMs?: number;
}

type Listener = (e: LiveJobEvent) => void;

const RECONCILE_EVERY_MS = 15 * 60_000;

const lazyReconcile = async (teamId: TeamId, db: Db) => (await import("./lazy-reconcile")).lazyReconcileTeam(teamId, db);

/**
 * Keeps the function alive for background work when the platform supports
 * it (Vercel's waitUntil; a no-op elsewhere). The promise runs either way and
 * is never awaited by the stream.
 */
async function background(task: Promise<unknown>): Promise<void> {
  const settled = task.catch(() => undefined);
  try {
    const { waitUntil } = await import("@vercel/functions");
    waitUntil(settled);
  } catch {
    // not on Vercel: the promise still runs
  }
}

export interface Poller {
  listeners: Set<Listener>;
  seen: Map<number, { version: number; at: number }>;
  /** Database time of the last poll (ms), or null before the first. */
  dbNow: number | null;
  idle: number;
  timer: ReturnType<typeof setTimeout> | null;
  stopped: boolean;
  base: number;
  max: number;
  lookback: number;
  /** When this instance may next ask for a lazy reconcile (ms); 0 = on the first poll. */
  nextReconcileAt: number;
}

// Per database (tests use several) and per team. The poll interval, look-back and reconcile hook are the first subscriber's.
const pollers = new WeakMap<Db, Map<string, Poller>>();
let defaultDb: Db | null = null;

/** How many team pollers are running against a database (for tests and diagnostics). */
export function activePollers(db: Db): number {
  return pollers.get(db)?.size ?? 0;
}

/** The poll query, with the database clock folded in: one round trip, and db_now even when nothing changed. */
const POLL = `
  SELECT n.db_now, x.*
  FROM (SELECT now() AS db_now) n
  LEFT JOIN (
    ${JOB_SELECT}
    WHERE j.row_updated_at > COALESCE(to_timestamp($2::double precision / 1000), now() - make_interval(secs => $3::double precision))
    ORDER BY j.version ASC LIMIT 1000
  ) x ON true
  ORDER BY x.version ASC NULLS FIRST`;

async function pollOnce(db: Db, teamId: TeamId, p: Poller): Promise<LiveJobEvent[]> {
  const since = p.dbNow === null ? null : p.dbNow - p.lookback;
  const rows = await db.query(POLL, [teamId, since, p.lookback / 1000]);
  const first = p.dbNow === null;
  p.dbNow = rows[0]?.db_now instanceof Date ? (rows[0].db_now as Date).getTime() : Date.now();
  const out: LiveJobEvent[] = [];
  for (const r of rows) {
    if (r.id == null) continue;
    const id = Number(r.id);
    const version = Number(r.version);
    const prev = p.seen.get(id);
    if (prev && prev.version >= version) continue;
    p.seen.set(id, { version, at: p.dbNow });
    // The first poll only learns what is already there.
    if (!first) out.push(jobEventFromRow(r));
  }
  for (const [id, s] of p.seen) if (s.at < p.dbNow - 4 * p.lookback) p.seen.delete(id);
  return out;
}

/** Next wait: the base interval while jobs change, then 5 s, then the ceiling. */
export function nextDelay(p: Pick<Poller, "idle" | "base" | "max">): number {
  if (p.idle < 3) return p.base;
  if (p.idle < 10) return Math.min(Math.max(p.base, 5000), p.max);
  return p.max;
}

/**
 * Subscribes to a team's job changes. All subscribers of a team in this
 * process share one poller: one query per interval per team, whatever the
 * number of viewers. The poller starts with the first subscriber and stops
 * with the last, so nobody watching means no queries, and Neon can scale to
 * zero (the pool closes idle connections after 20 s). Returns unsubscribe.
 */
export function subscribeTeamJobs(teamId: TeamId, listener: Listener, opts: PollOptions = {}): () => void {
  let unsubscribed = false;
  let poller: Poller | null = null;
  let db: Db | null = null;

  const start = async () => {
    db = opts.db ?? (defaultDb ??= await resolve());
    if (unsubscribed) return;
    let byTeam = pollers.get(db);
    if (!byTeam) pollers.set(db, (byTeam = new Map()));
    poller = byTeam.get(teamId) ?? null;
    if (!poller) {
      const base = Math.max(250, opts.intervalMs ?? 2000);
      const p: Poller = {
        listeners: new Set(),
        seen: new Map(),
        dbNow: null,
        idle: 0,
        timer: null,
        stopped: false,
        base,
        max: Math.max(base, opts.maxIntervalMs ?? 10000),
        lookback: Math.max(base * 2, opts.lookbackMs ?? 15000),
        nextReconcileAt: 0,
      };
      byTeam.set(teamId, p);
      poller = p;
      const d = db;
      const reconcileFn = opts.reconcile === undefined ? lazyReconcile : opts.reconcile;
      const every = Math.max(1000, opts.reconcileEveryMs ?? RECONCILE_EVERY_MS);
      const tick = async () => {
        if (p.stopped) return;
        try {
          const events = await pollOnce(d, teamId, p);
          p.idle = events.length ? 0 : p.idle + 1;
          for (const e of events) for (const l of p.listeners) l(e);
        } catch (e) {
          p.idle += 1;
          console.warn("[quod-cloud] live poll failed", { error: e instanceof Error ? e.name : "unknown" });
        }
        // Someone is watching: heal this team's lost deliveries and stuck jobs now and then, off the stream's path.
        if (reconcileFn && !p.stopped && Date.now() >= p.nextReconcileAt) {
          p.nextReconcileAt = Date.now() + every;
          void background(reconcileFn(teamId, d));
        }
        if (!p.stopped) p.timer = setTimeout(tick, nextDelay(p));
      };
      void tick();
    }
    poller.listeners.add(listener);
  };
  const ready = start();

  return () => {
    unsubscribed = true;
    void ready.then(() => {
      if (!poller || !db) return;
      poller.listeners.delete(listener);
      if (poller.listeners.size === 0) {
        poller.stopped = true;
        if (poller.timer) clearTimeout(poller.timer);
        pollers.get(db)?.delete(teamId);
      }
    });
  };
}

export interface ChangeFeedOptions extends PollOptions {
  signal: AbortSignal;
  /** Resume after this event id ("gh-<version>", from Last-Event-ID). Without it, only changes from now on. */
  after?: string | null;
}

/**
 * Job changes for a team, as events, until `signal` aborts: one consumer of
 * the team's shared poller (subscribeTeamJobs). With `after`, first the
 * changes since that event id (one query), then live ones, without repeats.
 *
 * A short poll on workflow_jobs.row_updated_at, not LISTEN/NOTIFY: Neon's
 * pooled connections (PgBouncer, transaction mode) cannot LISTEN, and a
 * direct connection held open per instance would keep Neon awake. Each poll
 * reads the rows changed in the look-back window (by database time) and
 * yields versions it has not yielded, so a write that commits after a later
 * one is still seen.
 */
export async function* onTeamJobChange(teamId: TeamId, opts: ChangeFeedOptions): AsyncGenerator<LiveJobEvent> {
  const { signal } = opts;
  if (signal.aborted) return;
  const queue: LiveJobEvent[] = [];
  let wake: (() => void) | null = null;
  const unsubscribe = subscribeTeamJobs(
    teamId,
    (e) => {
      if (queue.length >= 1000) queue.shift(); // a consumer that stopped reading: keep the latest
      queue.push(e);
      wake?.();
    },
    opts,
  );
  const onAbort = () => wake?.();
  signal.addEventListener("abort", onAbort);
  try {
    // Catch-up: what changed after the client's last event. Live events for the same job and version are skipped below.
    const resumed = new Map<string, number>();
    const after = versionOfEventId(opts.after);
    if (after !== null) {
      const q = opts.db ?? (await resolve());
      const rows = await q.query(`${JOB_SELECT} WHERE j.version > $2 ORDER BY j.version ASC LIMIT 1000`, [teamId, after]);
      for (const r of rows) {
        if (signal.aborted) return;
        resumed.set(String(r.id), Number(r.version));
        yield jobEventFromRow(r);
      }
    }
    while (!signal.aborted) {
      while (queue.length) {
        const e = queue.shift()!;
        const done = resumed.get(e.job.id);
        if (done !== undefined) {
          resumed.delete(e.job.id);
          if ((versionOfEventId(e.id) ?? 0) <= done) continue;
        }
        yield e;
        if (signal.aborted) return;
      }
      await new Promise<void>((r) => (wake = r));
      wake = null;
    }
  } finally {
    signal.removeEventListener("abort", onAbort);
    unsubscribe();
  }
}

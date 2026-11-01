import type { Db, Row } from "@/lib/db/types";
import { jsonArray } from "./live";
import type { GhStep } from "./payloads";
import type { TeamId } from "./team";

// Read models for the dashboard pages of a team with GitHub data (Runs, a
// GitHub run, Runners, Usage), from the GitHub App's tables. Same scoping as
// lib/github/live.ts: rows are visible only through an installation linked
// to the team (not deleted or suspended) and a repo still in it.

const TEAM_JOIN = `
  JOIN installations i ON i.id = w.installation_id AND i.team_id = $1 AND i.deleted_at IS NULL AND i.suspended_at IS NULL
  JOIN repos r ON r.id = w.repo_id AND r.installation_id = i.id AND r.removed_at IS NULL`;

const iso = (v: unknown): string | null => (v instanceof Date ? v.toISOString() : typeof v === "string" && v ? new Date(v).toISOString() : null);
const str = (v: unknown, fallback = ""): string => (typeof v === "string" ? v : fallback);
const secondsBetween = (a: string | null, b: string | null): number | null => (a && b ? Math.max(0, Math.round((Date.parse(b) - Date.parse(a)) / 1000)) : null);

/** What a run, job or step came to, in the dashboard's words. */
export type GhOutcome = "queued" | "running" | "passed" | "failed" | "cancelled" | "skipped";

export function ghOutcome(status: string, conclusion: string | null | undefined): GhOutcome {
  if (status === "in_progress") return "running";
  if (status !== "completed") return "queued";
  if (conclusion === "success") return "passed";
  if (conclusion === "cancelled") return "cancelled";
  if (conclusion === "skipped" || conclusion === "neutral") return "skipped";
  return "failed"; // failure, timed_out, startup_failure, action_required, stale
}

export const RUN_FILTERS = ["running", "queued", "passed", "failed", "cancelled"] as const;
export type RunFilter = (typeof RUN_FILTERS)[number];

// Fixed SQL per filter: user input never reaches the query text.
const STATUS_SQL: Record<RunFilter, string> = {
  running: "w.status = 'in_progress'",
  queued: "w.status_rank = 1",
  passed: "w.status = 'completed' AND w.conclusion = 'success'",
  failed: "w.status = 'completed' AND w.conclusion IN ('failure', 'timed_out', 'startup_failure', 'action_required', 'stale')",
  cancelled: "w.status = 'completed' AND w.conclusion = 'cancelled'",
};

export interface GhRun {
  id: number;
  repoId: number;
  repoFullName: string;
  workflow: string;
  title: string;
  /** Per workflow on GitHub, so not unique in a repo: runs are addressed by id. */
  runNumber: number | null;
  runAttempt: number;
  event: string;
  branch: string;
  sha: string;
  actor: string;
  status: "queued" | "in_progress" | "completed";
  conclusion: string | null;
  outcome: GhOutcome;
  htmlUrl: string;
  startedAt: string | null;
  finishedAt: string | null;
  durationSeconds: number | null;
}

const RUN_SELECT = `
  SELECT w.id, w.repo_id, r.full_name AS repo_full_name, w.name, w.display_title, w.run_number, w.run_attempt, w.event,
         w.head_branch, w.head_sha, w.actor_login, w.status, w.conclusion, w.html_url, w.gh_updated_at,
         COALESCE(w.run_started_at, w.created_at) AS started_at, t.finished
  FROM workflow_runs w
  ${TEAM_JOIN}
  LEFT JOIN LATERAL (
    SELECT max(j.completed_at) AS finished FROM workflow_jobs j WHERE j.run_id = w.id AND j.run_attempt = w.run_attempt
  ) t ON true`;

function runFromRow(row: Row): GhRun {
  const status = row.status === "completed" ? "completed" : row.status === "in_progress" ? "in_progress" : "queued";
  const conclusion = typeof row.conclusion === "string" ? row.conclusion : null;
  const startedAt = iso(row.started_at);
  // GitHub's updated_at of a completed run is when it finished, when no job says later.
  const finishedAt = status === "completed" ? (iso(row.finished) ?? iso(row.gh_updated_at)) : null;
  return {
    id: Number(row.id),
    repoId: Number(row.repo_id),
    repoFullName: str(row.repo_full_name),
    workflow: str(row.name),
    title: str(row.display_title, str(row.name)),
    runNumber: typeof row.run_number === "number" ? row.run_number : null,
    runAttempt: Number(row.run_attempt ?? 1),
    event: str(row.event),
    branch: str(row.head_branch),
    sha: str(row.head_sha),
    actor: str(row.actor_login),
    status,
    conclusion: status === "completed" ? conclusion : null,
    outcome: ghOutcome(status, conclusion),
    htmlUrl: str(row.html_url),
    startedAt,
    finishedAt,
    durationSeconds: secondsBetween(startedAt, finishedAt),
  };
}

export interface RunListOptions {
  repoId?: number | null;
  branch?: string | null;
  status?: RunFilter | null;
  /** 1-based. */
  page?: number;
  perPage?: number;
}

/** The team's workflow runs, newest first, one page; `hasMore` when another page follows. */
export async function listTeamRuns(teamId: TeamId, db: Db, opts: RunListOptions = {}): Promise<{ runs: GhRun[]; hasMore: boolean; page: number }> {
  const perPage = Math.min(Math.max(Math.trunc(opts.perPage ?? 25), 1), 100);
  const page = Math.min(Math.max(Math.trunc(opts.page ?? 1), 1), 1000);
  const params: unknown[] = [teamId, perPage + 1, (page - 1) * perPage];
  const where: string[] = [];
  if (opts.repoId) where.push(`w.repo_id = $${params.push(opts.repoId)}`);
  if (opts.branch) where.push(`w.head_branch = $${params.push(opts.branch.slice(0, 255))}`);
  if (opts.status) where.push(STATUS_SQL[opts.status]);
  const rows = await db.query(
    `${RUN_SELECT}
     ${where.length ? `WHERE ${where.join(" AND ")}` : ""}
     ORDER BY COALESCE(w.run_started_at, w.created_at, w.gh_updated_at) DESC, w.id DESC
     LIMIT $2 OFFSET $3`,
    params,
  );
  return { runs: rows.slice(0, perPage).map(runFromRow), hasMore: rows.length > perPage, page };
}

/** Does the team have any workflow run at all? (Empty states: "linked, nothing yet" vs "data".) */
export async function teamHasRuns(teamId: TeamId, db: Db): Promise<boolean> {
  const rows = await db.query(`SELECT 1 FROM workflow_runs w ${TEAM_JOIN} LIMIT 1`, [teamId]);
  return rows.length > 0;
}

export interface GhStepView {
  number: number;
  name: string;
  outcome: GhOutcome;
  startedAt: string | null;
  finishedAt: string | null;
  seconds: number | null;
}

export interface GhJobView {
  id: number;
  name: string;
  status: "queued" | "in_progress" | "completed";
  outcome: GhOutcome;
  startedAt: string | null;
  finishedAt: string | null;
  durationSeconds: number | null;
  runnerName: string | null;
  runnerGroup: string | null;
  labels: string[];
  htmlUrl: string;
  steps: GhStepView[];
}

/** One of the team's runs with its latest attempt's jobs and their steps; null if the team cannot see it. */
export async function getTeamRun(teamId: TeamId, db: Db, runId: number): Promise<{ run: GhRun; jobs: GhJobView[] } | null> {
  if (!Number.isSafeInteger(runId) || runId <= 0) return null;
  const [row] = await db.query(`${RUN_SELECT} WHERE w.id = $2`, [teamId, runId]);
  if (!row) return null;
  const run = runFromRow(row);
  const rows = await db.query(
    `SELECT id, name, status, conclusion, started_at, completed_at, created_at, runner_name, runner_group_name, labels, html_url, steps
     FROM workflow_jobs WHERE run_id = $1 AND run_attempt = $2
     ORDER BY COALESCE(started_at, created_at) ASC NULLS LAST, id ASC`,
    [run.id, run.runAttempt],
  );
  const jobs = rows.map((j): GhJobView => {
    const status = j.status === "completed" ? "completed" : j.status === "in_progress" ? "in_progress" : "queued";
    const startedAt = status === "queued" ? null : iso(j.started_at);
    const finishedAt = status === "completed" ? iso(j.completed_at) : null;
    const steps = (jsonArray(j.steps) as GhStep[])
      .filter((s) => s && typeof s.name === "string")
      .map((s) => {
        const sStart = iso(s.started_at);
        const sEnd = s.status === "completed" ? iso(s.completed_at) : null;
        return { number: Number(s.number), name: s.name, outcome: ghOutcome(s.status, s.conclusion), startedAt: sStart, finishedAt: sEnd, seconds: secondsBetween(sStart, sEnd) };
      })
      .sort((a, b) => a.number - b.number);
    return {
      id: Number(j.id),
      name: str(j.name),
      status,
      outcome: ghOutcome(status, typeof j.conclusion === "string" ? j.conclusion : null),
      startedAt,
      finishedAt,
      durationSeconds: secondsBetween(startedAt, finishedAt),
      runnerName: typeof j.runner_name === "string" && j.runner_name ? j.runner_name : null,
      runnerGroup: typeof j.runner_group_name === "string" && j.runner_group_name ? j.runner_group_name : null,
      labels: jsonArray(j.labels).map(String),
      htmlUrl: str(j.html_url),
      steps,
    };
  });
  return { run, jobs };
}

// ---------------------------------------------------------------- Runners seen in CI

export interface SeenRunner {
  key: string;
  /** A self-hosted runner's name, or a GitHub-hosted image's labels. */
  name: string;
  kind: "github-hosted" | "self-hosted";
  labels: string[];
  group: string | null;
  lastSeen: string;
  jobs24h: number;
  /** Jobs running on it now. */
  busy: number;
}

const HOSTED_NAME = /^GitHub Actions \d+$/;

/**
 * Runners seen in the team's jobs over the last `days` days. GitHub-hosted
 * runners are fresh machines with throwaway names, so they are grouped by
 * their labels (one row per image, e.g. ubuntu-latest); self-hosted ones by name.
 */
export async function teamRunners(teamId: TeamId, db: Db, opts: { now?: Date; days?: number } = {}): Promise<SeenRunner[]> {
  const now = opts.now ?? new Date();
  const rows = await db.query(
    `SELECT j.runner_name, j.runner_group_name, j.labels, j.status, j.started_at, j.completed_at
     FROM workflow_jobs j
     JOIN installations i ON i.id = j.installation_id AND i.team_id = $1 AND i.deleted_at IS NULL AND i.suspended_at IS NULL
     JOIN repos r ON r.id = j.repo_id AND r.installation_id = i.id AND r.removed_at IS NULL
     WHERE j.runner_name IS NOT NULL AND j.runner_name <> '' AND j.status_rank >= 2
       AND COALESCE(j.completed_at, j.started_at, j.row_updated_at) > $2::timestamptz - make_interval(days => $3)
     ORDER BY COALESCE(j.completed_at, j.started_at) DESC NULLS LAST
     LIMIT 5000`,
    [teamId, now.toISOString(), opts.days ?? 7],
  );
  const day = now.getTime() - 24 * 3600_000;
  const out = new Map<string, SeenRunner>();
  for (const r of rows) {
    const name = str(r.runner_name);
    const group = typeof r.runner_group_name === "string" && r.runner_group_name ? r.runner_group_name : null;
    const labels = jsonArray(r.labels).map(String);
    const hosted = !labels.includes("self-hosted") && (group === "GitHub Actions" || HOSTED_NAME.test(name));
    const key = hosted ? `hosted:${[...labels].sort().join(",")}` : `self:${name}`;
    const seen = iso(r.completed_at) ?? iso(r.started_at) ?? now.toISOString();
    let s = out.get(key);
    if (!s) {
      s = { key, name: hosted ? labels.join(", ") || "GitHub-hosted" : name, kind: hosted ? "github-hosted" : "self-hosted", labels, group, lastSeen: seen, jobs24h: 0, busy: 0 };
      out.set(key, s);
    }
    if (seen > s.lastSeen) s.lastSeen = seen;
    const started = iso(r.started_at);
    if (started && Date.parse(started) >= day) s.jobs24h += 1;
    if (r.status === "in_progress") s.busy += 1;
  }
  return [...out.values()].sort((a, b) => b.busy - a.busy || b.lastSeen.localeCompare(a.lastSeen));
}

// ---------------------------------------------------------------- CI minutes (job time)

export interface JobMinutes {
  /** YYYY-MM-DD (UTC) -> minutes of job time that finished that day, for each of the last `days` days, oldest first. */
  days: { day: string; minutes: number; jobs: number }[];
  totalMinutes: number;
  jobs: number;
}

/** Job time (started to completed) of the team's finished jobs, per UTC day, over the last `days` days. */
export async function teamJobMinutes(teamId: TeamId, db: Db, opts: { now?: Date; days?: number } = {}): Promise<JobMinutes> {
  const now = opts.now ?? new Date();
  const days = Math.min(Math.max(Math.trunc(opts.days ?? 7), 1), 90);
  const rows = await db.query<{ day: string; seconds: number | string; jobs: number | string }>(
    `SELECT to_char(j.completed_at AT TIME ZONE 'UTC', 'YYYY-MM-DD') AS day,
            sum(EXTRACT(EPOCH FROM (j.completed_at - j.started_at))) AS seconds, count(*) AS jobs
     FROM workflow_jobs j
     JOIN installations i ON i.id = j.installation_id AND i.team_id = $1 AND i.deleted_at IS NULL AND i.suspended_at IS NULL
     JOIN repos r ON r.id = j.repo_id AND r.installation_id = i.id AND r.removed_at IS NULL
     WHERE j.status = 'completed' AND j.started_at IS NOT NULL AND j.completed_at >= j.started_at
       AND j.completed_at >= date_trunc('day', $2::timestamptz AT TIME ZONE 'UTC') AT TIME ZONE 'UTC' - make_interval(days => $3 - 1)
     GROUP BY 1`,
    [teamId, now.toISOString(), days],
  );
  const byDay = new Map(rows.map((r) => [r.day, { minutes: Number(r.seconds) / 60, jobs: Number(r.jobs) }]));
  const out: JobMinutes["days"] = [];
  for (let i = days - 1; i >= 0; i--) {
    const d = new Date(now.getTime() - i * 86_400_000).toISOString().slice(0, 10);
    const v = byDay.get(d);
    out.push({ day: d, minutes: Math.round(v?.minutes ?? 0), jobs: v?.jobs ?? 0 });
  }
  return { days: out, totalMinutes: out.reduce((a, d) => a + d.minutes, 0), jobs: out.reduce((a, d) => a + d.jobs, 0) };
}

// ---------------------------------------------------------------- URL filters

export interface RunQuery {
  repoSlug: string | null;
  repoId: number | null;
  branch: string | null;
  status: RunFilter | null;
  page: number;
}

const first = (v: string | string[] | undefined): string | undefined => (Array.isArray(v) ? v[0] : v);

/** ?repo=&branch=&status=&page= from the URL. An unknown repo or status is ignored, not an error. */
export function parseRunQuery(sp: Record<string, string | string[] | undefined>, repos: { id: number; slug: string }[] = []): RunQuery {
  const repo = repos.find((r) => r.slug === first(sp.repo)) ?? null;
  const branch = first(sp.branch)?.trim().slice(0, 255) || null;
  const s = first(sp.status);
  const status = (RUN_FILTERS as readonly string[]).includes(s ?? "") ? (s as RunFilter) : null;
  const p = Number(first(sp.page));
  return { repoSlug: repo?.slug ?? null, repoId: repo?.id ?? null, branch, status, page: Number.isInteger(p) && p >= 1 && p <= 1000 ? p : 1 };
}

import type { Queryable } from "@/lib/db/types";
import type { GhAccount, GhInstallation, GhRepo, JobRow, RunRow } from "./payloads";

// Writes from webhooks and from installation linking. Every write to a run
// or job is monotonic: a delivery only applies when its (attempt, status
// rank, timestamp) is not older than what is stored, so a late "queued" or
// "in_progress" never overwrites "completed". Applied changes take a new
// live_version, which the live layer streams.

type RepoRef = Pick<GhRepo, "id" | "name" | "full_name" | "private">;

const json = (v: unknown) => JSON.stringify(v);

// ---------------------------------------------------------------- Deliveries

/** Records a delivery id. False when it was seen before (a redelivery). */
export async function recordDelivery(q: Queryable, id: string, event: string): Promise<boolean> {
  const rows = await q.query("INSERT INTO deliveries (id, event) VALUES ($1, $2) ON CONFLICT (id) DO NOTHING RETURNING id", [id, event]);
  return rows.length === 1;
}

/** Deletes delivery ids older than 7 days. Returns how many. */
export async function pruneDeliveries(q: Queryable): Promise<number> {
  const rows = await q.query("DELETE FROM deliveries WHERE received_at < now() - interval '7 days' RETURNING id");
  return rows.length;
}

// ---------------------------------------------------------------- Installations and repos

export async function upsertInstallation(q: Queryable, inst: GhInstallation): Promise<void> {
  const a = inst.account ?? null;
  await q.query(
    `INSERT INTO installations (id, account_id, account_login, account_type, repository_selection, suspended_at)
     VALUES ($1, $2, $3, $4, $5, $6::timestamptz)
     ON CONFLICT (id) DO UPDATE SET
       account_id = COALESCE(EXCLUDED.account_id, installations.account_id),
       account_login = COALESCE(EXCLUDED.account_login, installations.account_login),
       account_type = COALESCE(EXCLUDED.account_type, installations.account_type),
       repository_selection = COALESCE(EXCLUDED.repository_selection, installations.repository_selection),
       updated_at = now()`,
    [inst.id, a?.id ?? null, a?.login ?? null, a?.type ?? null, inst.repository_selection ?? null, inst.suspended_at ?? null],
  );
}

/** A row for an installation first seen through a run or job delivery (installed before Quod stored anything). */
export async function ensureInstallation(q: Queryable, id: number, owner: GhAccount | undefined): Promise<void> {
  await q.query(
    `INSERT INTO installations (id, account_id, account_login, account_type) VALUES ($1, $2, $3, $4) ON CONFLICT (id) DO NOTHING`,
    [id, owner?.id ?? null, owner?.login ?? null, owner?.type ?? null],
  );
}

/** Uninstalled: unlink it from its team. Its runs and jobs stay, but no team can see them. */
export async function deleteInstallation(q: Queryable, id: number): Promise<void> {
  await q.query(
    `UPDATE installations SET deleted_at = now(), team_id = NULL, linked_by = NULL, linked_by_github_id = NULL, linked_at = NULL, updated_at = now() WHERE id = $1`,
    [id],
  );
  await q.query("UPDATE repos SET removed_at = now(), updated_at = now() WHERE installation_id = $1 AND removed_at IS NULL", [id]);
}

export async function setSuspended(q: Queryable, id: number, suspended: boolean): Promise<void> {
  await q.query(`UPDATE installations SET suspended_at = ${suspended ? "now()" : "NULL"}, updated_at = now() WHERE id = $1`, [id]);
}

/**
 * Repos now in the installation. `restore` clears removed_at (the repo was
 * added again); run and job deliveries pass false so a late delivery never
 * brings back a removed repo.
 */
export async function upsertRepos(q: Queryable, installationId: number, repos: RepoRef[], restore = true): Promise<void> {
  for (const r of repos) {
    await q.query(
      `INSERT INTO repos (id, installation_id, full_name, name, private) VALUES ($1, $2, $3, $4, $5)
       ON CONFLICT (id) DO UPDATE SET
         installation_id = EXCLUDED.installation_id, full_name = EXCLUDED.full_name, name = EXCLUDED.name, private = EXCLUDED.private,
         ${restore ? "removed_at = NULL," : ""} updated_at = now()`,
      [r.id, installationId, r.full_name, r.name, r.private ?? true],
    );
  }
}

export async function removeRepos(q: Queryable, installationId: number, repoIds: number[]): Promise<void> {
  if (!repoIds.length) return;
  await q.query(
    `UPDATE repos SET removed_at = now(), updated_at = now()
     WHERE installation_id = $1 AND removed_at IS NULL AND id IN (SELECT (jsonb_array_elements_text($2::jsonb))::bigint)`,
    [installationId, json(repoIds)],
  );
}

// ---------------------------------------------------------------- Runs and jobs

/** Upserts a run if the delivery is not older than the stored row. Returns the applied version, or null. */
export async function upsertRun(q: Queryable, r: RunRow): Promise<{ version: number; inserted: boolean } | null> {
  const rows = await q.query<{ version: number; inserted: boolean }>(
    `INSERT INTO workflow_runs (id, installation_id, repo_id, name, display_title, run_number, run_attempt, event, head_branch, head_sha,
       actor_login, status, conclusion, status_rank, html_url, run_started_at, created_at, gh_updated_at)
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16::timestamptz, $17::timestamptz, $18::timestamptz)
     ON CONFLICT (id) DO UPDATE SET
       installation_id = EXCLUDED.installation_id,
       repo_id = EXCLUDED.repo_id,
       name = COALESCE(EXCLUDED.name, workflow_runs.name),
       display_title = COALESCE(EXCLUDED.display_title, workflow_runs.display_title),
       run_number = COALESCE(EXCLUDED.run_number, workflow_runs.run_number),
       run_attempt = EXCLUDED.run_attempt,
       event = COALESCE(EXCLUDED.event, workflow_runs.event),
       head_branch = COALESCE(EXCLUDED.head_branch, workflow_runs.head_branch),
       head_sha = COALESCE(EXCLUDED.head_sha, workflow_runs.head_sha),
       actor_login = COALESCE(EXCLUDED.actor_login, workflow_runs.actor_login),
       status = EXCLUDED.status,
       conclusion = EXCLUDED.conclusion,
       status_rank = EXCLUDED.status_rank,
       html_url = COALESCE(EXCLUDED.html_url, workflow_runs.html_url),
       run_started_at = COALESCE(EXCLUDED.run_started_at, workflow_runs.run_started_at),
       created_at = COALESCE(EXCLUDED.created_at, workflow_runs.created_at),
       gh_updated_at = EXCLUDED.gh_updated_at,
       version = nextval('live_version'),
       row_updated_at = now()
     WHERE (EXCLUDED.run_attempt, EXCLUDED.status_rank, EXCLUDED.gh_updated_at)
           >= (workflow_runs.run_attempt, workflow_runs.status_rank, workflow_runs.gh_updated_at)
       AND (EXCLUDED.run_attempt, EXCLUDED.status, EXCLUDED.conclusion, EXCLUDED.gh_updated_at, EXCLUDED.run_started_at)
           IS DISTINCT FROM (workflow_runs.run_attempt, workflow_runs.status, workflow_runs.conclusion, workflow_runs.gh_updated_at, workflow_runs.run_started_at)
     RETURNING version, (xmax = 0) AS inserted`,
    [
      r.id, r.installation_id, r.repo_id, r.name, r.display_title, r.run_number, r.run_attempt, r.event, r.head_branch, r.head_sha,
      r.actor_login, r.status, r.conclusion, r.status_rank, r.html_url, r.run_started_at, r.created_at, r.gh_updated_at,
    ],
  );
  const row = rows[0];
  if (!row) return null;
  // The run's number and actor are now known: re-announce its jobs that arrived first.
  if (row.inserted) await q.query("UPDATE workflow_jobs SET version = nextval('live_version'), row_updated_at = now() WHERE run_id = $1", [r.id]);
  return { version: Number(row.version), inserted: Boolean(row.inserted) };
}

/** Upserts a job if the delivery is not older than the stored row. Returns the applied version, or null. */
export async function upsertJob(q: Queryable, j: JobRow): Promise<number | null> {
  const rows = await q.query<{ version: number }>(
    `INSERT INTO workflow_jobs (id, run_id, installation_id, repo_id, run_attempt, workflow_name, name, head_branch, head_sha, sender_login,
       status, conclusion, status_rank, labels, runner_name, runner_group_name, html_url, created_at, started_at, completed_at, steps, event_at)
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14::jsonb, $15, $16, $17,
       $18::timestamptz, $19::timestamptz, $20::timestamptz, $21::jsonb, $22::timestamptz)
     ON CONFLICT (id) DO UPDATE SET
       run_id = EXCLUDED.run_id,
       installation_id = EXCLUDED.installation_id,
       repo_id = EXCLUDED.repo_id,
       run_attempt = EXCLUDED.run_attempt,
       workflow_name = COALESCE(EXCLUDED.workflow_name, workflow_jobs.workflow_name),
       name = EXCLUDED.name,
       head_branch = COALESCE(EXCLUDED.head_branch, workflow_jobs.head_branch),
       head_sha = COALESCE(EXCLUDED.head_sha, workflow_jobs.head_sha),
       sender_login = COALESCE(workflow_jobs.sender_login, EXCLUDED.sender_login),
       status = EXCLUDED.status,
       conclusion = EXCLUDED.conclusion,
       status_rank = EXCLUDED.status_rank,
       labels = CASE WHEN jsonb_array_length(EXCLUDED.labels) > 0 THEN EXCLUDED.labels ELSE workflow_jobs.labels END,
       runner_name = COALESCE(EXCLUDED.runner_name, workflow_jobs.runner_name),
       runner_group_name = COALESCE(EXCLUDED.runner_group_name, workflow_jobs.runner_group_name),
       html_url = COALESCE(EXCLUDED.html_url, workflow_jobs.html_url),
       created_at = COALESCE(workflow_jobs.created_at, EXCLUDED.created_at),
       started_at = COALESCE(EXCLUDED.started_at, workflow_jobs.started_at),
       completed_at = COALESCE(EXCLUDED.completed_at, workflow_jobs.completed_at),
       steps = CASE WHEN jsonb_array_length(EXCLUDED.steps) > 0 THEN EXCLUDED.steps ELSE workflow_jobs.steps END,
       event_at = EXCLUDED.event_at,
       version = nextval('live_version'),
       row_updated_at = now()
     WHERE (EXCLUDED.run_attempt, EXCLUDED.status_rank, EXCLUDED.event_at)
           >= (workflow_jobs.run_attempt, workflow_jobs.status_rank, workflow_jobs.event_at)
       AND (EXCLUDED.run_attempt, EXCLUDED.status, EXCLUDED.conclusion, EXCLUDED.event_at, EXCLUDED.steps, EXCLUDED.runner_name, EXCLUDED.started_at, EXCLUDED.completed_at)
           IS DISTINCT FROM (workflow_jobs.run_attempt, workflow_jobs.status, workflow_jobs.conclusion, workflow_jobs.event_at, workflow_jobs.steps, workflow_jobs.runner_name, workflow_jobs.started_at, workflow_jobs.completed_at)
     RETURNING version`,
    [
      j.id, j.run_id, j.installation_id, j.repo_id, j.run_attempt, j.workflow_name, j.name, j.head_branch, j.head_sha, j.sender_login,
      j.status, j.conclusion, j.status_rank, json(j.labels), j.runner_name, j.runner_group_name, j.html_url,
      j.created_at, j.started_at, j.completed_at, json(j.steps), j.event_at,
    ],
  );
  return rows[0] ? Number(rows[0].version) : null;
}

// ---------------------------------------------------------------- Linking

export interface LinkInput {
  installation: GhInstallation;
  teamId: string;
  /** WorkOS user id of the team admin. */
  userId: string;
  githubUserId: number;
  repos: RepoRef[];
}

/**
 * Links an installation to a team, in one transaction. Refuses (returns
 * "linked-elsewhere") when another team already has it. The repo list is the
 * one verified with GitHub during linking; others are marked removed.
 */
export async function linkInstallation(q: Queryable, input: LinkInput): Promise<"linked" | "linked-elsewhere" | "deleted"> {
  await upsertInstallation(q, input.installation);
  const [row] = await q.query<{ team_id: string | null; deleted_at: Date | null }>("SELECT team_id, deleted_at FROM installations WHERE id = $1 FOR UPDATE", [input.installation.id]);
  if (row?.deleted_at) return "deleted";
  if (row?.team_id && row.team_id !== input.teamId) return "linked-elsewhere";
  await q.query(
    `UPDATE installations SET team_id = $2, linked_by = $3, linked_by_github_id = $4, linked_at = now(), updated_at = now() WHERE id = $1`,
    [input.installation.id, input.teamId, input.userId, input.githubUserId],
  );
  await upsertRepos(q, input.installation.id, input.repos);
  await q.query(
    `UPDATE repos SET removed_at = now(), updated_at = now()
     WHERE installation_id = $1 AND removed_at IS NULL AND id NOT IN (SELECT (jsonb_array_elements_text($2::jsonb))::bigint)`,
    [input.installation.id, json(input.repos.map((r) => r.id))],
  );
  return "linked";
}

/** Which team an installation is linked to, if any. */
export async function installationTeam(q: Queryable, installationId: number): Promise<string | null> {
  const [row] = await q.query<{ team_id: string | null }>("SELECT team_id FROM installations WHERE id = $1 AND deleted_at IS NULL", [installationId]);
  return row?.team_id ?? null;
}

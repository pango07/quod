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

/** Retention: delivery ids after 7 days, runs and jobs not updated for 30 days. */
export async function pruneOld(q: Queryable): Promise<{ deliveries: number; runs: number; jobs: number }> {
  const deliveries = (await q.query("DELETE FROM deliveries WHERE received_at < now() - interval '7 days' RETURNING id")).length;
  const jobs = (await q.query("DELETE FROM workflow_jobs WHERE row_updated_at < now() - interval '30 days' RETURNING id")).length;
  const runs = (await q.query("DELETE FROM workflow_runs WHERE row_updated_at < now() - interval '30 days' RETURNING id")).length;
  return { deliveries, runs, jobs };
}

// ---------------------------------------------------------------- Installations and repos

/**
 * Applies an installation delivery if it is not older than the last one
 * applied (GitHub's installation.updated_at, else the delivery time).
 * `suspend` sets or clears suspended_at with it. Returns whether it applied.
 */
export async function upsertInstallation(q: Queryable, inst: GhInstallation, at: Date, suspend?: "suspend" | "unsuspend"): Promise<boolean> {
  const a = inst.account ?? null;
  const eventAt = inst.updated_at ? new Date(inst.updated_at) : at;
  const rows = await q.query(
    `INSERT INTO installations (id, account_id, account_login, account_type, repository_selection, suspended_at, gh_updated_at)
     VALUES ($1, $2, $3, $4, $5, CASE WHEN $7 = 'suspend' THEN COALESCE($6::timestamptz, now()) END, $8::timestamptz)
     ON CONFLICT (id) DO UPDATE SET
       account_id = COALESCE(EXCLUDED.account_id, installations.account_id),
       account_login = COALESCE(EXCLUDED.account_login, installations.account_login),
       account_type = COALESCE(EXCLUDED.account_type, installations.account_type),
       repository_selection = COALESCE(EXCLUDED.repository_selection, installations.repository_selection),
       suspended_at = CASE WHEN $7 = 'suspend' THEN COALESCE($6::timestamptz, now()) WHEN $7 = 'unsuspend' THEN NULL ELSE installations.suspended_at END,
       gh_updated_at = EXCLUDED.gh_updated_at,
       updated_at = now()
     WHERE installations.gh_updated_at IS NULL OR EXCLUDED.gh_updated_at >= installations.gh_updated_at
     RETURNING id`,
    [inst.id, a?.id ?? null, a?.login ?? null, a?.type ?? null, inst.repository_selection ?? null, inst.suspended_at ?? null, suspend ?? "", eventAt],
  );
  return rows.length === 1;
}

/**
 * A row for an installation first seen through a run or job delivery, or
 * through linking. Fills only what is missing; never overrides a webhook's values.
 */
export async function ensureInstallation(q: Queryable, id: number, owner: GhAccount | null | undefined, repositorySelection?: string): Promise<void> {
  await q.query(
    `INSERT INTO installations (id, account_id, account_login, account_type, repository_selection) VALUES ($1, $2, $3, $4, $5)
     ON CONFLICT (id) DO UPDATE SET
       account_id = COALESCE(installations.account_id, EXCLUDED.account_id),
       account_login = COALESCE(installations.account_login, EXCLUDED.account_login),
       account_type = COALESCE(installations.account_type, EXCLUDED.account_type),
       repository_selection = COALESCE(installations.repository_selection, EXCLUDED.repository_selection)`,
    [id, owner?.id ?? null, owner?.login ?? null, owner?.type ?? null, repositorySelection ?? null],
  );
}

/** installation_target renamed: the account's new login. */
export async function renameAccount(q: Queryable, installationId: number, account: GhAccount): Promise<void> {
  await q.query("UPDATE installations SET account_login = $2, account_id = COALESCE(account_id, $3), updated_at = now() WHERE id = $1", [installationId, account.login, account.id ?? null]);
  // repos.full_name changes with the owner's login.
  await q.query("UPDATE repos SET full_name = $2 || '/' || name, updated_at = now() WHERE installation_id = $1 AND split_part(full_name, '/', 1) <> $2", [installationId, account.login]);
}

/** The reconcile sweep asked GitHub about this run now. */
export async function markReconciled(q: Queryable, runId: number): Promise<void> {
  await q.query("UPDATE workflow_runs SET reconciled_at = now() WHERE id = $1", [runId]);
}

/** Uninstalled: unlink it from its team. Its runs and jobs stay, but no team can see them. */
export async function deleteInstallation(q: Queryable, id: number): Promise<void> {
  await q.query(
    `UPDATE installations SET deleted_at = now(), team_id = NULL, linked_by = NULL, linked_by_github_id = NULL, linked_at = NULL, updated_at = now() WHERE id = $1`,
    [id],
  );
  await q.query("UPDATE repos SET removed_at = now(), updated_at = now() WHERE installation_id = $1 AND removed_at IS NULL", [id]);
}

/**
 * Repos joining the installation at `at` (installation created,
 * installation_repositories added, linking): restored unless a later
 * delivery removed them. Applies only if not older than the repo's last
 * membership change (m2).
 */
export async function addRepos(q: Queryable, installationId: number, repos: RepoRef[], at: Date): Promise<void> {
  for (const r of repos) {
    await q.query(
      `INSERT INTO repos (id, installation_id, full_name, name, private, membership_at) VALUES ($1, $2, $3, $4, $5, $6::timestamptz)
       ON CONFLICT (id) DO UPDATE SET
         installation_id = EXCLUDED.installation_id, full_name = EXCLUDED.full_name, name = EXCLUDED.name, private = EXCLUDED.private,
         removed_at = NULL, membership_at = EXCLUDED.membership_at, updated_at = now()
       WHERE repos.membership_at IS NULL OR EXCLUDED.membership_at >= repos.membership_at`,
      [r.id, installationId, r.full_name, r.name, r.private ?? true, at],
    );
  }
}

/**
 * The repo of a run or job delivery: created if unknown, names kept fresh,
 * membership untouched (a late delivery never brings back a removed repo).
 */
export async function touchRepo(q: Queryable, installationId: number, r: RepoRef): Promise<void> {
  await q.query(
    `INSERT INTO repos (id, installation_id, full_name, name, private) VALUES ($1, $2, $3, $4, $5)
     ON CONFLICT (id) DO UPDATE SET full_name = EXCLUDED.full_name, name = EXCLUDED.name, private = EXCLUDED.private, updated_at = now()
     WHERE repos.installation_id = EXCLUDED.installation_id`,
    [r.id, installationId, r.full_name, r.name, r.private ?? true],
  );
}

/** Repos leaving the installation at `at`, unless a later delivery added them back. */
export async function removeRepos(q: Queryable, installationId: number, repoIds: number[], at: Date): Promise<void> {
  if (!repoIds.length) return;
  await q.query(
    `UPDATE repos SET removed_at = now(), membership_at = $3::timestamptz, updated_at = now()
     WHERE installation_id = $1 AND id IN (SELECT (jsonb_array_elements_text($2::jsonb))::bigint)
       AND (membership_at IS NULL OR membership_at <= $3::timestamptz)`,
    [installationId, json(repoIds), at],
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
  await ensureInstallation(q, input.installation.id, input.installation.account, input.installation.repository_selection);
  const now = new Date();
  const [row] = await q.query<{ team_id: string | null; deleted_at: Date | null }>("SELECT team_id, deleted_at FROM installations WHERE id = $1 FOR UPDATE", [input.installation.id]);
  if (row?.deleted_at) return "deleted";
  if (row?.team_id && row.team_id !== input.teamId) return "linked-elsewhere";
  await q.query(
    `UPDATE installations SET team_id = $2, linked_by = $3, linked_by_github_id = $4, linked_at = now(), updated_at = now() WHERE id = $1`,
    [input.installation.id, input.teamId, input.userId, input.githubUserId],
  );
  await addRepos(q, input.installation.id, input.repos, now);
  await q.query(
    `UPDATE repos SET removed_at = now(), membership_at = $3::timestamptz, updated_at = now()
     WHERE installation_id = $1 AND removed_at IS NULL AND id NOT IN (SELECT (jsonb_array_elements_text($2::jsonb))::bigint)`,
    [input.installation.id, json(input.repos.map((r) => r.id)), now],
  );
  return "linked";
}

/** A team admin's Disconnect: the installation stays on GitHub, but this team no longer sees it. Only the team that has it can do this. */
export async function disconnectInstallation(q: Queryable, teamId: string, installationId: number): Promise<boolean> {
  const rows = await q.query(
    "UPDATE installations SET team_id = NULL, linked_by = NULL, linked_by_github_id = NULL, linked_at = NULL, updated_at = now() WHERE id = $1 AND team_id = $2 RETURNING id",
    [installationId, teamId],
  );
  return rows.length === 1;
}

/** Which team an installation is linked to, if any. */
export async function installationTeam(q: Queryable, installationId: number): Promise<string | null> {
  const [row] = await q.query<{ team_id: string | null }>("SELECT team_id FROM installations WHERE id = $1 AND deleted_at IS NULL", [installationId]);
  return row?.team_id ?? null;
}

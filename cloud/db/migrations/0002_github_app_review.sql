-- 0002: fixes from the Fable review of the GitHub App (2633568..ab031b3).
--
-- installations.gh_updated_at: GitHub's installation.updated_at of the last
--   applied installation delivery, so a late one never undoes a newer one (m2).
-- repos.membership_at: when the repo last joined or left the installation,
--   by the delivery's time, so a late "added" never revives a removed repo (m2).
-- workflow_runs.reconciled_at: when the reconcile cron last asked GitHub for
--   this run's state; the sweep checks the longest-unchecked runs first (M2).

ALTER TABLE installations ADD COLUMN gh_updated_at timestamptz;
ALTER TABLE repos ADD COLUMN membership_at timestamptz;
ALTER TABLE workflow_runs ADD COLUMN reconciled_at timestamptz;

-- The sweep: unfinished runs, oldest-checked first.
CREATE INDEX workflow_runs_unfinished_idx ON workflow_runs (COALESCE(reconciled_at, row_updated_at)) WHERE status_rank < 3;
-- Unfinished jobs whose run row never arrived.
CREATE INDEX workflow_jobs_unfinished_idx ON workflow_jobs (row_updated_at) WHERE status_rank < 3;
-- 30-day retention of runs (workflow_jobs already has a row_updated_at index).
CREATE INDEX workflow_runs_row_updated_idx ON workflow_runs (row_updated_at);

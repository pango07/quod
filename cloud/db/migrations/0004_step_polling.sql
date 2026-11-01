-- 0004: live step progress for running jobs.
--
-- GitHub sends workflow_job webhooks only when a job is queued, starts and
-- completes, never per step. While someone watches a team, its live poller
-- reads that team's running jobs back from GitHub's REST API about every
-- 15 s (lib/github/step-poll.ts).
--
-- workflow_jobs.steps_polled_at: when an instance last claimed this job for a
--   step poll. The claim is one conditional UPDATE (older than the interval,
--   FOR UPDATE SKIP LOCKED), so across instances each job is read at most
--   once per interval, and the least recently read go first.
-- Rate-limit back-off reuses reconcile_state: key "gh-backoff:<installation
--   id>", whose last_run_at is then the time the back-off ends.

ALTER TABLE workflow_jobs ADD COLUMN steps_polled_at timestamptz;

-- Running jobs, least recently polled first.
CREATE INDEX workflow_jobs_running_idx ON workflow_jobs (installation_id, steps_polled_at NULLS FIRST) WHERE status = 'in_progress';

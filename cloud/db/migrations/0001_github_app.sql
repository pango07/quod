-- 0001: the Quod CI GitHub App. Installations and their repos, live CI status
-- from workflow_run / workflow_job webhooks, and webhook delivery ids for dedupe.
--
-- Applied by `npm run db:migrate` (cloud/scripts/db-migrate.mjs), which runs
-- each file once, in a transaction, and records it in schema_migrations.
-- No secrets live in these tables: the App's private key, webhook secret and
-- client secret are environment variables only, and user OAuth tokens are
-- used once during linking and never stored.
--
-- Ids are GitHub's (bigint). team_id is the WorkOS organization id of the
-- Quod team an installation is linked to (null: not linked yet).

-- Increases on every change a webhook makes to a run or job. The live layer
-- uses it as the event id and the poll cursor.
CREATE SEQUENCE live_version;

CREATE TABLE installations (
  id                   bigint PRIMARY KEY,
  account_id           bigint,
  account_login        text,
  -- "User" or "Organization"
  account_type         text,
  -- "all" or "selected"
  repository_selection text,
  team_id              text,
  -- WorkOS user id and GitHub user id of the team admin who linked it
  linked_by            text,
  linked_by_github_id  bigint,
  linked_at            timestamptz,
  suspended_at         timestamptz,
  deleted_at           timestamptz,
  created_at           timestamptz NOT NULL DEFAULT now(),
  updated_at           timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX installations_team_idx ON installations (team_id) WHERE team_id IS NOT NULL;

CREATE TABLE repos (
  id              bigint PRIMARY KEY,
  installation_id bigint NOT NULL,
  -- owner/name
  full_name       text NOT NULL,
  name            text NOT NULL,
  private         boolean NOT NULL DEFAULT true,
  -- set when the repo leaves the installation
  removed_at      timestamptz,
  created_at      timestamptz NOT NULL DEFAULT now(),
  updated_at      timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX repos_installation_idx ON repos (installation_id);

CREATE TABLE workflow_runs (
  id              bigint PRIMARY KEY,
  installation_id bigint NOT NULL,
  repo_id         bigint NOT NULL,
  name            text,
  display_title   text,
  run_number      integer,
  run_attempt     integer NOT NULL DEFAULT 1,
  event           text,
  head_branch     text,
  head_sha        text,
  actor_login     text,
  -- requested | in_progress | completed | queued | waiting | pending
  status          text NOT NULL,
  conclusion      text,
  -- 1 queued-like, 2 in_progress, 3 completed: updates never lower it within an attempt
  status_rank     smallint NOT NULL,
  html_url        text,
  run_started_at  timestamptz,
  -- GitHub's created_at / updated_at for the run
  created_at      timestamptz,
  gh_updated_at   timestamptz NOT NULL,
  version         bigint NOT NULL DEFAULT nextval('live_version'),
  row_updated_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX workflow_runs_installation_idx ON workflow_runs (installation_id, run_started_at DESC);
CREATE INDEX workflow_runs_repo_idx ON workflow_runs (repo_id, run_started_at DESC);

CREATE TABLE workflow_jobs (
  id                bigint PRIMARY KEY,
  run_id            bigint NOT NULL,
  installation_id   bigint NOT NULL,
  repo_id           bigint NOT NULL,
  run_attempt       integer NOT NULL DEFAULT 1,
  workflow_name     text,
  name              text NOT NULL,
  head_branch       text,
  head_sha          text,
  -- login of the delivery's sender (the run's actor is on workflow_runs)
  sender_login      text,
  -- queued | waiting | in_progress | completed (and requested / pending)
  status            text NOT NULL,
  conclusion        text,
  status_rank       smallint NOT NULL,
  -- runs-on labels, a JSON array of strings
  labels            jsonb NOT NULL DEFAULT '[]'::jsonb,
  runner_name       text,
  runner_group_name text,
  html_url          text,
  created_at        timestamptz,
  started_at        timestamptz,
  completed_at      timestamptz,
  -- the payload's steps[]: name, status, conclusion, number, started_at, completed_at
  steps             jsonb NOT NULL DEFAULT '[]'::jsonb,
  -- the latest of the payload's timestamps: orders deliveries of the same rank
  event_at          timestamptz NOT NULL,
  version           bigint NOT NULL DEFAULT nextval('live_version'),
  row_updated_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX workflow_jobs_installation_status_idx ON workflow_jobs (installation_id, status_rank, event_at DESC);
CREATE INDEX workflow_jobs_run_idx ON workflow_jobs (run_id);
CREATE INDEX workflow_jobs_row_updated_idx ON workflow_jobs (row_updated_at);

-- One row per X-GitHub-Delivery, so a redelivery is applied once. Pruned after 7 days.
CREATE TABLE deliveries (
  id          text PRIMARY KEY,
  event       text NOT NULL,
  received_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX deliveries_received_idx ON deliveries (received_at);

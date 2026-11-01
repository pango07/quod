-- 0003: a throttle for GitHub reconciliation (Vercel Hobby: one cron a day).
--
-- While someone watches a team's dashboard, its live poller asks to
-- reconcile that team at most every 15 minutes. Several function instances
-- may poll the same team, so the claim is one atomic statement on this row
-- (key "team:<WorkOS org id>"): insert, or update only when last_run_at is
-- older than the interval, RETURNING tells the one instance that won.

CREATE TABLE reconcile_state (
  key         text PRIMARY KEY,
  last_run_at timestamptz NOT NULL
);

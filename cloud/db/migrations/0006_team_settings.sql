-- 0006: per-team settings kept by the dashboard. For now one: the runner
-- price per minute that the Overview's money saved is estimated with
-- (lib/team-settings.ts). Unset (no row, or before this migration is
-- applied), the Overview uses GitHub's published Linux 2-core price.
--
-- team_id: the team's WorkOS organization id. No secrets are stored.

CREATE TABLE team_settings (
  team_id               text PRIMARY KEY,
  runner_usd_per_minute numeric(10, 6) CHECK (runner_usd_per_minute > 0 AND runner_usd_per_minute <= 1),
  updated_by            text,
  updated_at            timestamptz NOT NULL DEFAULT now()
);

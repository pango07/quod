-- 0005: default-branch protection of linked repos, for the writer-trust
-- warning (DECISIONS.md 2026-11-02: trusted writes from an unprotected default
-- branch are allowed, and warned about on the team and the affected repos).
--
-- A cache of what the GitHub App last read (lib/github/protection.ts): the
-- repository's default branch, its classic branch protection (Administration:
-- read) and whether any ruleset rule applies to it. Read lazily while someone
-- views the team, at most every 6 hours per repo, and again soon after a
-- branch_protection_rule, repository_ruleset or repository webhook marks the
-- row stale. No tokens or secrets are stored.
--
-- state:      protected | unprotected | unknown (not read yet, no permission, or an error)
-- classic:    classic protection on the branch (null: could not read it)
-- rulesets:   at least one ruleset rule applies to the branch (null: could not read it)
-- reason:     why the state is unknown, or a part could not be read (no-permission, no-access, ...)
-- checked_at: the last completed read (null: never)
-- stale:      a webhook said it may have changed: read again on the next view
-- claimed_at: an instance is reading it now; a claim older than 2 minutes is abandoned

CREATE TABLE repo_protection (
  repo_id         bigint PRIMARY KEY,
  installation_id bigint NOT NULL,
  default_branch  text,
  state           text NOT NULL DEFAULT 'unknown' CHECK (state IN ('protected', 'unprotected', 'unknown')),
  classic         boolean,
  rulesets        boolean,
  reason          text,
  checked_at      timestamptz,
  stale           boolean NOT NULL DEFAULT false,
  claimed_at      timestamptz,
  updated_at      timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX repo_protection_installation_idx ON repo_protection (installation_id);

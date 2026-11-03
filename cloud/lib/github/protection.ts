import type { BranchProtection } from "@/lib/api/types";
import type { Db, Queryable } from "@/lib/db/types";
import { PLAN_REASON, type ProtectionRead } from "./api";
import type { TeamId } from "./team";

// Default-branch protection of a team's linked repos, for the writer-trust
// warning (DECISIONS.md 2026-11-02). Trusted results are written by jobs on
// the default branch; when anyone who can push there can do that, the team
// and the repo say so.
//
// Cached in repo_protection (db/migrations/0005_branch_protection.sql):
// - Read while someone views the team: rows missing, stale (a webhook said
//   so) or older than 6 h are refreshed in the background (never awaited by
//   the page), at most `MAX_PER_PASS` repos per view.
// - Each repo is claimed with one conditional upsert, so several instances
//   never read the same repo at once, and a claim older than 2 minutes is
//   abandoned.
// - Before the migration is applied, or when the database or GitHub does not
//   answer, every repo is "unknown" and nothing is written.

export const REFRESH_AFTER_S = 6 * 3600;
export const MAX_PER_PASS = 10;
const CLAIM_TIMEOUT_S = 120;

const iso = (v: unknown): string | null => (v instanceof Date ? v.toISOString() : typeof v === "string" && v ? new Date(v).toISOString() : null);

/** What a read says: protected when classic protection or a ruleset applies; unprotected only when both were read and neither does. */
export function protectionState(r: { classic: boolean | null; rulesets: boolean | null }): BranchProtection["state"] {
  if (r.classic === true || r.rulesets === true) return "protected";
  if (r.classic === false && r.rulesets === false) return "unprotected";
  return "unknown";
}

export interface ProtectionGitHub {
  read(installationId: number, fullName: string): Promise<ProtectionRead>;
}

/** Is the cache table there? (Not until 0005 is applied.) */
export async function protectionTableExists(q: Queryable): Promise<boolean> {
  const [r] = await q.query<{ ok: boolean }>("SELECT to_regclass('repo_protection') IS NOT NULL AS ok");
  return !!r?.ok;
}

interface CacheRow {
  repo_id: number;
  state: BranchProtection["state"];
  default_branch: string | null;
  reason: string | null;
  checked_at: unknown;
  /** Never read, and either no row yet (a read starts now) or claimed within the last PENDING_S. */
  pending: boolean;
  due: boolean;
}

/** How long a first read may show "checking protection" before it says "unknown" (the read itself times out at 5 s). */
export const PENDING_S = 10;

/** The cached row as the pages show it. */
export function cachedProtection(r: Pick<CacheRow, "repo_id" | "state" | "default_branch" | "reason" | "checked_at" | "pending">): BranchProtection {
  const state = r.repo_id === null ? "unknown" : r.state;
  return {
    state,
    branch: r.default_branch ?? null,
    checkedAt: iso(r.checked_at),
    ...(state === "unknown" && r.reason === PLAN_REASON ? { detail: "plan" as const } : {}),
    ...(r.pending && !iso(r.checked_at) ? { pending: true } : {}),
  };
}

/**
 * The cached protection of `repos` (the team's linked repos, by GitHub id),
 * and the ids due for a read; `byRepo` is null before the migration is applied. Repos the team cannot see are never read: the
 * query joins through the team's installations, as lib/github/live.ts does.
 */
export async function readCached(db: Queryable, teamId: TeamId, repoIds: number[], now = new Date()): Promise<{ byRepo: Map<number, BranchProtection> | null; due: number[] }> {
  const byRepo = new Map<number, BranchProtection>();
  // Before 0005 is applied there is nothing to show: null, and no badge at all (not "unknown" everywhere).
  if (!(await protectionTableExists(db))) return { byRepo: null, due: [] };
  if (!repoIds.length) return { byRepo, due: [] };
  const rows = await db.query<CacheRow & { id: number }>(
    `SELECT r.id, p.repo_id, p.state, p.default_branch, p.reason, p.checked_at,
            (p.checked_at IS NULL AND (p.repo_id IS NULL OR p.claimed_at >= $3::timestamptz - make_interval(secs => $5))) AS pending,
            (p.repo_id IS NULL OR p.stale OR p.checked_at IS NULL OR p.checked_at < $3::timestamptz - make_interval(secs => $4)) AS due
     FROM repos r
     JOIN installations i ON i.id = r.installation_id AND i.team_id = $1 AND i.deleted_at IS NULL AND i.suspended_at IS NULL
     LEFT JOIN repo_protection p ON p.repo_id = r.id
     WHERE r.removed_at IS NULL AND r.id = ANY($2::bigint[])`,
    [teamId, repoIds, now.toISOString(), REFRESH_AFTER_S, PENDING_S],
  );
  const due: number[] = [];
  for (const r of rows) {
    const id = Number(r.id);
    byRepo.set(id, cachedProtection(r));
    if (r.due) due.push(id);
  }
  return { byRepo, due };
}

/**
 * Claims up to `limit` of `repoIds` for a read: due (missing, stale or older
 * than 6 h) and not claimed by another instance in the last 2 minutes.
 */
export async function claimForRefresh(q: Queryable, teamId: TeamId, repoIds: number[], opts: { now?: Date; limit?: number } = {}): Promise<{ id: number; installation_id: number; full_name: string }[]> {
  const now = (opts.now ?? new Date()).toISOString();
  if (!repoIds.length) return [];
  const candidates = await q.query<{ id: number; installation_id: number; full_name: string }>(
    `SELECT r.id, r.installation_id, r.full_name
     FROM repos r
     JOIN installations i ON i.id = r.installation_id AND i.team_id = $1 AND i.deleted_at IS NULL AND i.suspended_at IS NULL
     LEFT JOIN repo_protection p ON p.repo_id = r.id
     WHERE r.removed_at IS NULL AND r.id = ANY($2::bigint[])
       AND (p.repo_id IS NULL OR p.stale OR p.checked_at IS NULL OR p.checked_at < $3::timestamptz - make_interval(secs => $4))
     ORDER BY p.checked_at ASC NULLS FIRST, r.id
     LIMIT $5`,
    [teamId, repoIds, now, REFRESH_AFTER_S, opts.limit ?? MAX_PER_PASS],
  );
  const claimed: { id: number; installation_id: number; full_name: string }[] = [];
  for (const c of candidates) {
    const won = await q.query(
      `INSERT INTO repo_protection (repo_id, installation_id, claimed_at) VALUES ($1, $2, $3::timestamptz)
       ON CONFLICT (repo_id) DO UPDATE SET claimed_at = EXCLUDED.claimed_at, installation_id = EXCLUDED.installation_id
       WHERE (repo_protection.claimed_at IS NULL OR repo_protection.claimed_at < $3::timestamptz - make_interval(secs => $4))
         AND (repo_protection.stale OR repo_protection.checked_at IS NULL OR repo_protection.checked_at < $3::timestamptz - make_interval(secs => $5))
       RETURNING repo_id`,
      [c.id, c.installation_id, now, CLAIM_TIMEOUT_S, REFRESH_AFTER_S],
    );
    if (won.length) claimed.push({ id: Number(c.id), installation_id: Number(c.installation_id), full_name: c.full_name });
  }
  return claimed;
}

/** Records one read (or that it could not be made) and releases the claim. */
export async function recordRead(q: Queryable, repoId: number, read: ProtectionRead, now = new Date()): Promise<void> {
  if (read.kind === "limited") {
    // Try again after the claim times out; keep what we had.
    return;
  }
  if (read.kind === "error") {
    // A GitHub hiccup or the 5 s timeout: keep what was known, and a repo never read becomes "unknown" (not
    // "checking" forever). Marked stale, so the next view reads it again once the claim lapses (2 minutes).
    await q.query(
      `UPDATE repo_protection SET reason = $2, checked_at = COALESCE(checked_at, $3::timestamptz), stale = true, updated_at = now() WHERE repo_id = $1`,
      [repoId, read.reason === "timeout" ? "timeout" : `error-${read.status}`, now.toISOString()],
    );
    return;
  }
  if (read.kind === "gone") {
    await q.query(`UPDATE repo_protection SET state = 'unknown', reason = 'gone', checked_at = $2::timestamptz, stale = false, claimed_at = NULL, updated_at = now() WHERE repo_id = $1`, [repoId, now.toISOString()]);
    return;
  }
  await q.query(
    `UPDATE repo_protection SET state = $2, default_branch = $3, classic = $4, rulesets = $5, reason = $6, checked_at = $7::timestamptz, stale = false, claimed_at = NULL, updated_at = now()
     WHERE repo_id = $1`,
    [repoId, protectionState(read), read.defaultBranch, read.classic, read.rulesets, read.reason, now.toISOString()],
  );
}

/** One background pass: claim the due repos, read each from GitHub, record. Stops at a rate limit. Never throws. */
export async function refreshProtection(db: Db, teamId: TeamId, repoIds: number[], gh: ProtectionGitHub, now = new Date()): Promise<{ read: number; limited: boolean }> {
  let read = 0;
  try {
    if (!(await protectionTableExists(db))) return { read, limited: false };
    const claimed = await claimForRefresh(db, teamId, repoIds, { now });
    for (const c of claimed) {
      const r = await gh.read(c.installation_id, c.full_name).catch((): ProtectionRead => ({ kind: "error", status: 0, rate: { remaining: null, resetAt: null, retryAfterMs: null } }));
      await recordRead(db, c.id, r, now);
      if (r.kind === "limited") return { read, limited: true };
      read++;
    }
  } catch (e) {
    console.warn("[quod-cloud] branch protection refresh failed", { error: e instanceof Error ? e.name : "unknown" });
  }
  return { read, limited: false };
}

/**
 * Marks repos' protection stale after a webhook (branch_protection_rule,
 * repository_ruleset, repository): the next view reads them again. `repoId`
 * null with an installation: every repo of it (an organization ruleset).
 * A no-op before the migration is applied.
 */
export async function markProtectionStale(q: Queryable, target: { repoId: number | null; installationId: number | null }): Promise<number> {
  if (!(await protectionTableExists(q))) return 0;
  if (target.repoId !== null) return (await q.query("UPDATE repo_protection SET stale = true WHERE repo_id = $1 RETURNING repo_id", [target.repoId])).length;
  if (target.installationId !== null) return (await q.query("UPDATE repo_protection SET stale = true WHERE installation_id = $1 RETURNING repo_id", [target.installationId])).length;
  return 0;
}

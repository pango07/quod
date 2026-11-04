import "server-only";
import { dbConfigured, getDb, type Db } from "@/lib/db";
import { DEFAULT_USD_PER_MINUTE, parseRate } from "@/lib/savings";

// Per-team settings kept in the dashboard's database (team_settings,
// db/migrations/0006_team_settings.sql). Today: the runner price the
// Overview estimates money saved with. Every read degrades to the default:
// no database, the migration not applied yet, or the database down.

export interface RunnerRate {
  usdPerMinute: number;
  /** "team": the team set it; "default": GitHub's Linux 2-core price. */
  source: "team" | "default";
  /** Whether a team admin can save one here (the database and table exist). */
  canSave: boolean;
}

const DEFAULT: RunnerRate = { usdPerMinute: DEFAULT_USD_PER_MINUTE, source: "default", canSave: false };

async function table(db: Db): Promise<boolean> {
  const [r] = await db.query<{ ok: boolean }>("SELECT to_regclass('team_settings') IS NOT NULL AS ok");
  return !!r?.ok;
}

/** The team's runner price per minute (`teamId`: the WorkOS organization id). */
export async function runnerRate(teamId: string, given?: Db): Promise<RunnerRate> {
  if (!given && !dbConfigured()) return DEFAULT;
  try {
    const db = given ?? (await getDb());
    if (!(await table(db))) return DEFAULT;
    const [row] = await db.query<{ rate: string | number | null }>("SELECT runner_usd_per_minute AS rate FROM team_settings WHERE team_id = $1", [teamId]);
    const rate = parseRate(row?.rate ?? null);
    return rate === null ? { ...DEFAULT, canSave: true } : { usdPerMinute: rate, source: "team", canSave: true };
  } catch (e) {
    console.error("team settings: read failed", e instanceof Error ? e.message : e);
    return DEFAULT;
  }
}

/** Saves (or, with null, clears back to the default) the team's runner price. Throws when it cannot. */
export async function saveRunnerRate(teamId: string, rate: number | null, by: string, given?: Db): Promise<void> {
  const db = given ?? (await getDb());
  if (!(await table(db))) throw new Error("team_settings is not set up on this deploy (migration 0006).");
  await db.query(
    `INSERT INTO team_settings (team_id, runner_usd_per_minute, updated_by, updated_at) VALUES ($1, $2, $3, now())
     ON CONFLICT (team_id) DO UPDATE SET runner_usd_per_minute = EXCLUDED.runner_usd_per_minute, updated_by = EXCLUDED.updated_by, updated_at = now()`,
    [teamId, rate, by],
  );
}

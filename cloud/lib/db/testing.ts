// Test helper: a fresh in-memory PGlite with every migration applied.
import { fileURLToPath } from "node:url";
import { loadMigrations, migrate } from "../../db/migrate.mjs";
import { openPglite } from "./pglite";
import type { Db } from "./types";

export const MIGRATIONS_DIR = fileURLToPath(new URL("../../db/migrations", import.meta.url));

export async function migratedDb(): Promise<Db> {
  const db = await openPglite();
  await migrate(db, await loadMigrations(MIGRATIONS_DIR));
  return db;
}

/** Empties the app tables (keeps schema_migrations and the sequence). */
export async function truncateAll(db: Db): Promise<void> {
  await db.exec("TRUNCATE installations, repos, workflow_runs, workflow_jobs, deliveries");
}

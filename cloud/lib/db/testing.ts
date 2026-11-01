// Test helper: a fresh database with every migration applied.
//
// By default an in-memory PGlite. With TEST_DATABASE_URL set, a real Postgres
// through lib/db/postgres.ts (the production driver, postgres.js), in a schema
// of its own per call, dropped on close. The two drivers encode parameters
// differently (a45df10: jsonb), so CI should run the suite both ways: give
// the test job a Postgres service and set TEST_DATABASE_URL, e.g.
//   TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres npm test
// Never point it at a database that matters: tests create and drop schemas.
import { randomBytes } from "node:crypto";
import { fileURLToPath } from "node:url";
import { loadMigrations, migrate } from "../../db/migrate.mjs";
import { openPglite } from "./pglite";
import { postgresDb } from "./postgres";
import type { Db } from "./types";

export const MIGRATIONS_DIR = fileURLToPath(new URL("../../db/migrations", import.meta.url));

/** Which backend migratedDb() gives: "postgres" when TEST_DATABASE_URL is set. */
export const testBackend = (): "postgres" | "pglite" => (process.env.TEST_DATABASE_URL ? "postgres" : "pglite");

export async function migratedDb(): Promise<Db> {
  const url = process.env.TEST_DATABASE_URL;
  const db = url ? await postgresInOwnSchema(url) : await openPglite();
  await migrate(db, await loadMigrations(MIGRATIONS_DIR));
  return db;
}

async function postgresInOwnSchema(url: string): Promise<Db> {
  const schema = `quod_test_${randomBytes(6).toString("hex")}`;
  const admin = postgresDb(url, { max: 1 });
  await admin.exec(`CREATE SCHEMA ${schema}`);
  const db = postgresDb(url, { searchPath: schema });
  return {
    ...db,
    close: async () => {
      await db.close();
      try {
        await admin.exec(`DROP SCHEMA ${schema} CASCADE`);
      } finally {
        await admin.close();
      }
    },
  };
}

/** Empties the app tables (keeps schema_migrations and the sequence). */
export async function truncateAll(db: Db): Promise<void> {
  await db.exec("TRUNCATE installations, repos, workflow_runs, workflow_jobs, deliveries, reconcile_state");
}

// The migration runner's core, shared by scripts/db-migrate.mjs and the tests.
// Plain JavaScript so `npm run db:migrate` needs no build step.
//
// Each cloud/db/migrations/NNNN_name.sql runs once, in its own transaction,
// and is recorded in schema_migrations. A file that was applied and then
// edited is refused (its checksum no longer matches): write a new file instead.

import { createHash } from "node:crypto";
import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";

const NAME = /^\d{4}_[a-z0-9_]+\.sql$/;
// pg_advisory_xact_lock key: two runners never apply migrations at once.
const LOCK = 7243118001;

/**
 * @typedef {{ query: (text: string, params?: unknown[]) => Promise<Record<string, unknown>[]>, exec: (text: string) => Promise<void> }} Queryable
 * @typedef {Queryable & { tx: <T>(fn: (q: Queryable) => Promise<T>) => Promise<T> }} Db
 * @typedef {{ version: string, file: string, sql: string, checksum: string }} Migration
 */

/** @param {string} dir @returns {Promise<Migration[]>} */
export async function loadMigrations(dir) {
  const files = (await readdir(dir)).filter((f) => f.endsWith(".sql")).sort();
  const out = [];
  for (const file of files) {
    if (!NAME.test(file)) throw new Error(`Migration file name must look like 0001_name.sql: ${file}`);
    const sql = await readFile(join(dir, file), "utf8");
    out.push({ version: file.slice(0, 4), file, sql, checksum: createHash("sha256").update(sql).digest("hex") });
  }
  const versions = out.map((m) => m.version);
  if (new Set(versions).size !== versions.length) throw new Error("Two migration files share a version number.");
  return out;
}

const ENSURE = `CREATE TABLE IF NOT EXISTS schema_migrations (
  version    text PRIMARY KEY,
  file       text NOT NULL,
  checksum   text NOT NULL,
  applied_at timestamptz NOT NULL DEFAULT now()
)`;

/**
 * The migrations not applied yet. Read-only: it does not create schema_migrations.
 * @param {Db} db @param {Migration[]} migrations @returns {Promise<Migration[]>}
 */
export async function pendingMigrations(db, migrations) {
  const [exists] = await db.query("SELECT to_regclass('schema_migrations') IS NOT NULL AS ok");
  const rows = exists?.ok ? await db.query("SELECT version, file, checksum FROM schema_migrations") : [];
  const applied = new Map(rows.map((r) => [String(r.version), r]));
  for (const m of migrations) {
    const a = applied.get(m.version);
    if (a && a.checksum !== m.checksum) throw new Error(`${m.file} was changed after it was applied. Add a new migration instead.`);
  }
  return migrations.filter((m) => !applied.has(m.version));
}

/**
 * Applies every pending migration, each in its own transaction.
 * @param {Db} db @param {Migration[]} migrations
 * @param {{ log?: (line: string) => void }} [opts]
 * @returns {Promise<string[]>} the files applied
 */
export async function migrate(db, migrations, opts = {}) {
  const log = opts.log ?? (() => {});
  await db.exec(ENSURE);
  const pending = await pendingMigrations(db, migrations);
  /** @type {string[]} */
  const done = [];
  for (const m of pending) {
    await db.tx(async (q) => {
      await q.query("SELECT pg_advisory_xact_lock($1)", [LOCK]);
      // Another runner may have applied it while this one waited for the lock.
      const again = await q.query("SELECT 1 FROM schema_migrations WHERE version = $1", [m.version]);
      if (again.length) return;
      await q.exec(m.sql);
      await q.query("INSERT INTO schema_migrations (version, file, checksum) VALUES ($1, $2, $3)", [m.version, m.file, m.checksum]);
      done.push(m.file);
      log(`applied ${m.file}`);
    });
  }
  return done;
}

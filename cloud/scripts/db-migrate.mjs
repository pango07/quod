#!/usr/bin/env node
// npm run db:migrate [-- --dry-run] [-- --pglite <dir>]
//
// Applies cloud/db/migrations/*.sql that are not yet in schema_migrations.
// Target: DATABASE_URL_UNPOOLED if set (direct connection, preferred for
// DDL), else DATABASE_URL. With --pglite <dir>, a local PGlite folder instead
// (for `next dev` with DATABASE_URL=pglite:<dir>).
//
// It prints the target host and the pending files before applying anything.
// --dry-run prints them and stops.

import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { loadMigrations, migrate, pendingMigrations } from "../db/migrate.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const dryRun = args.includes("--dry-run");
const pgliteAt = args.indexOf("--pglite");
const pgliteDir = pgliteAt >= 0 ? args[pgliteAt + 1] : null;
if (pgliteAt >= 0 && !pgliteDir) fail("--pglite needs a folder, e.g. --pglite ./.pglite");

function fail(message) {
  console.error(`db:migrate: ${message}`);
  process.exit(1);
}

async function openTarget() {
  if (pgliteDir) {
    const { PGlite } = await import("@electric-sql/pglite");
    const pg = new PGlite(pgliteDir);
    await pg.waitReady;
    const wrap = (q) => ({ query: async (t, p = []) => (await q.query(t, p)).rows, exec: async (t) => void (await q.exec(t)) });
    return { label: `PGlite folder ${pgliteDir}`, db: { ...wrap(pg), tx: (fn) => pg.transaction((tx) => fn(wrap(tx))) }, close: () => pg.close() };
  }
  const url = process.env.DATABASE_URL_UNPOOLED?.trim() || process.env.DATABASE_URL?.trim();
  if (!url) fail("set DATABASE_URL_UNPOOLED (or DATABASE_URL), or pass --pglite <dir>.");
  let parsed;
  try {
    parsed = new URL(url);
  } catch {
    fail("the database URL is not a URL.");
  }
  const { default: postgres } = await import("postgres");
  const sql = postgres(url, { max: 1, prepare: false, connect_timeout: 15, onnotice: () => {} });
  const wrap = (s) => ({ query: (t, p = []) => s.unsafe(t, p), exec: async (t) => void (await s.unsafe(t)) });
  // Never print the password.
  return { label: `${parsed.hostname}${parsed.pathname}`, db: { ...wrap(sql), tx: (fn) => sql.begin((tx) => fn(wrap(tx))) }, close: () => sql.end({ timeout: 5 }) };
}

const migrations = await loadMigrations(join(here, "..", "db", "migrations"));
const target = await openTarget();
try {
  console.log(`db:migrate: target ${target.label}`);
  const pending = await pendingMigrations(target.db, migrations);
  if (!pending.length) {
    console.log("db:migrate: up to date, nothing to apply.");
  } else {
    console.log(`db:migrate: pending: ${pending.map((m) => m.file).join(", ")}`);
    if (dryRun) {
      console.log("db:migrate: --dry-run, nothing applied.");
    } else {
      const done = await migrate(target.db, migrations, { log: (l) => console.log(`db:migrate: ${l}`) });
      console.log(`db:migrate: done (${done.length} applied).`);
    }
  }
} catch (e) {
  console.error(`db:migrate: failed: ${e instanceof Error ? e.message : String(e)}`);
  process.exitCode = 1;
} finally {
  await target.close();
}

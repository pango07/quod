import "server-only";
import { postgresDb } from "./postgres";
import type { Db } from "./types";

export type { Db, Queryable, Row } from "./types";

// The app's database, from DATABASE_URL (Neon's pooled URL on Vercel).
// `next dev` may instead use a PGlite folder: DATABASE_URL=pglite:./.pglite
// (run `npm run db:migrate -- --pglite ./.pglite` first). Production refuses
// the pglite: form.

export class DbNotConfiguredError extends Error {
  constructor(message = "DATABASE_URL is not set.") {
    super(message);
    this.name = "DbNotConfiguredError";
  }
}

const g = globalThis as unknown as { __quodDb?: Promise<Db> };

export function dbConfigured(env = process.env): boolean {
  return Boolean(env.DATABASE_URL?.trim());
}

/** The shared Db for this process (one small pool, reused across requests). */
export function getDb(): Promise<Db> {
  return (g.__quodDb ??= open().catch((e) => {
    g.__quodDb = undefined;
    throw e;
  }));
}

async function open(): Promise<Db> {
  const url = process.env.DATABASE_URL?.trim();
  if (!url) throw new DbNotConfiguredError();
  if (url.startsWith("pglite:")) {
    if (process.env.NODE_ENV === "production") throw new DbNotConfiguredError("DATABASE_URL=pglite: is for local development only.");
    const { openPglite } = await import("./pglite");
    return openPglite(url.slice("pglite:".length) || undefined);
  }
  return postgresDb(url);
}

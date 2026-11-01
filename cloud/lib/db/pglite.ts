import type { PGlite } from "@electric-sql/pglite";
import type { Db, Queryable, Row } from "./types";

// In-process Postgres (PGlite, WASM) behind the Db seam: the tests use it in
// memory, and `next dev` can use it from a folder (DATABASE_URL=pglite:./.pglite).
// Never used in production: the package is a dev dependency.

interface PgliteLike {
  query<T>(text: string, params?: unknown[]): Promise<{ rows: T[] }>;
  exec(text: string): Promise<unknown>;
}

/** int8 comes back as BigInt from PGlite; the app wants numbers like the postgres adapter gives. */
function normalize<T>(rows: T[]): T[] {
  for (const row of rows as Row[]) {
    for (const k of Object.keys(row)) {
      const v = row[k];
      if (typeof v === "bigint") row[k] = Number(v);
    }
  }
  return rows;
}

function wrap(q: PgliteLike): Queryable {
  return {
    async query<T = Row>(text: string, params: unknown[] = []) {
      return normalize((await q.query<T>(text, params)).rows);
    },
    async exec(text: string) {
      await q.exec(text);
    },
  };
}

export function pgliteDb(pg: PGlite): Db {
  // PGlite runs one statement at a time; a second transaction waits for the first.
  return {
    ...wrap(pg),
    tx: (fn) => pg.transaction((tx) => fn(wrap(tx))),
    close: () => pg.close(),
  };
}

/** A fresh in-memory database, or one persisted in `dataDir`. */
export async function openPglite(dataDir?: string): Promise<Db> {
  const { PGlite } = await import("@electric-sql/pglite");
  const pg = dataDir ? new PGlite(dataDir) : new PGlite();
  await pg.waitReady;
  return pgliteDb(pg);
}

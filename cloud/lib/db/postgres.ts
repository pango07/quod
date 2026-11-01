import postgres from "postgres";
import type { Db, Queryable, Row } from "./types";

// The production adapter: the `postgres` driver over TCP, which works in
// Vercel's Node.js functions. DATABASE_URL is Neon's pooled (PgBouncer,
// transaction mode) URL, so prepared statements are off. One small pool per
// function instance, reused across invocations.

type Sql = postgres.Sql | postgres.TransactionSql;

/**
 * Callers pass jsonb parameters as JSON text (`$2::jsonb` with JSON.stringify),
 * which PGlite parses as given. The `postgres` driver instead asks the server
 * for each parameter's type and, for jsonb, JSON-encodes the value again: the
 * array `[1,2]` would be stored as the string "[1,2]". Casting through text
 * keeps the parameter a plain string, so both drivers store the same thing.
 */
export function jsonbAsText(text: string): string {
  return text.replace(/(\$\d+)::jsonb\b/g, "$1::text::jsonb");
}

function wrap(sql: Sql): Queryable {
  return {
    async query<T = Row>(text: string, params: unknown[] = []) {
      const rows = await sql.unsafe(jsonbAsText(text), params as postgres.ParameterOrJSON<never>[]);
      return rows as unknown as T[];
    },
    async exec(text: string) {
      await sql.unsafe(text);
    },
  };
}

export function postgresDb(url: string, opts: { max?: number } = {}): Db {
  const sql = postgres(url, {
    max: opts.max ?? 3,
    prepare: false,
    idle_timeout: 20,
    connect_timeout: 10,
    // int8 as a number: GitHub ids and counts fit in 2^53.
    types: { int8: { to: 20, from: [20], serialize: (x: number) => String(x), parse: (x: string) => Number(x) } },
    onnotice: () => {},
  });
  return {
    ...wrap(sql),
    tx: async <T>(fn: (q: Queryable) => Promise<T>) => (await sql.begin((tx) => fn(wrap(tx)))) as T,
    close: () => sql.end({ timeout: 5 }),
  };
}

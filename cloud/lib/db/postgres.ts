import postgres from "postgres";
import type { Db, Queryable, Row } from "./types";

// The production adapter: the `postgres` driver over TCP, which works in
// Vercel's Node.js functions. DATABASE_URL is Neon's pooled (PgBouncer,
// transaction mode) URL, so prepared statements are off. One small pool per
// function instance, reused across invocations.

type Sql = postgres.Sql | postgres.TransactionSql;

function wrap(sql: Sql): Queryable {
  return {
    async query<T = Row>(text: string, params: unknown[] = []) {
      const rows = await sql.unsafe(text, params as postgres.ParameterOrJSON<never>[]);
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

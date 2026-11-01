// The database seam. Two adapters implement it: postgres.ts (the `postgres`
// driver, for Neon on Vercel) and pglite.ts (in-process Postgres, for tests
// and local dev). Rows come back with int8 as a JS number (GitHub ids fit in
// 2^53), timestamptz as Date and jsonb parsed.

export type Row = Record<string, unknown>;

export interface Queryable {
  /** One parameterized statement ($1, $2, ...). */
  query<T = Row>(text: string, params?: unknown[]): Promise<T[]>;
  /** One or more statements without parameters (migrations). */
  exec(text: string): Promise<void>;
}

export interface Db extends Queryable {
  /** Runs fn in a transaction: committed if it resolves, rolled back if it throws. */
  tx<T>(fn: (q: Queryable) => Promise<T>): Promise<T>;
  close(): Promise<void>;
}

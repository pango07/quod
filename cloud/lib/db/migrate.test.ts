import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { loadMigrations, migrate, pendingMigrations } from "../../db/migrate.mjs";
import { openPglite } from "./pglite";
import { MIGRATIONS_DIR } from "./testing";

describe("migrations (PGlite)", () => {
  it("apply cleanly to an empty database, then are a no-op", async () => {
    const db = await openPglite();
    const all = await loadMigrations(MIGRATIONS_DIR);
    expect(all.length).toBeGreaterThan(0);
    expect(await migrate(db, all)).toEqual(all.map((m) => m.file));
    expect(await migrate(db, all)).toEqual([]);
    const tables = (await db.query<{ t: string }>("SELECT table_name AS t FROM information_schema.tables WHERE table_schema = 'public' ORDER BY 1")).map((r) => r.t);
    expect(tables).toEqual(["deliveries", "installations", "repos", "schema_migrations", "workflow_jobs", "workflow_runs"]);
    await db.close();
  });

  it("lists pending migrations without writing anything (the --dry-run path)", async () => {
    const db = await openPglite();
    const all = await loadMigrations(MIGRATIONS_DIR);
    expect((await pendingMigrations(db, all)).map((m) => m.file)).toEqual(all.map((m) => m.file));
    const [{ ok }] = await db.query<{ ok: boolean }>("SELECT to_regclass('schema_migrations') IS NOT NULL AS ok");
    expect(ok).toBe(false);
    await db.close();
  });

  it("refuses a migration that was edited after it was applied", async () => {
    const dir = await mkdtemp(join(tmpdir(), "quod-mig-"));
    const first = await readFile(join(MIGRATIONS_DIR, "0001_github_app.sql"), "utf8");
    await writeFile(join(dir, "0001_github_app.sql"), first);
    const db = await openPglite();
    await migrate(db, await loadMigrations(dir));
    await writeFile(join(dir, "0001_github_app.sql"), `${first}\n-- edited\n`);
    await expect(migrate(db, await loadMigrations(dir))).rejects.toThrow(/changed after it was applied/);
    await db.close();
  });

  it("rolls a failing migration back entirely", async () => {
    const dir = await mkdtemp(join(tmpdir(), "quod-mig-"));
    await writeFile(join(dir, "0001_bad.sql"), "CREATE TABLE half (id int); SELECT no_such_function();");
    const db = await openPglite();
    await expect(migrate(db, await loadMigrations(dir))).rejects.toThrow();
    const [{ ok }] = await db.query<{ ok: boolean }>("SELECT to_regclass('half') IS NOT NULL AS ok");
    expect(ok).toBe(false);
    expect(await db.query("SELECT * FROM schema_migrations")).toEqual([]);
    await db.close();
  });
});

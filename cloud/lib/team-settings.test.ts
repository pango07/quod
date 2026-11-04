import { describe, expect, it, vi } from "vitest";

vi.mock("server-only", () => ({}));

import { openPglite } from "./db/pglite";
import { migratedDb } from "./db/testing";
import { DEFAULT_USD_PER_MINUTE } from "./savings";
import { runnerRate, saveRunnerRate } from "./team-settings";

describe("the team's runner price", () => {
  it("is GitHub's Linux 2-core price until a team sets one, then theirs; empty goes back", async () => {
    const db = await migratedDb();
    expect(await runnerRate("org_a", db)).toEqual({ usdPerMinute: DEFAULT_USD_PER_MINUTE, source: "default", canSave: true });
    await saveRunnerRate("org_a", 0.016, "user_1", db);
    expect(await runnerRate("org_a", db)).toEqual({ usdPerMinute: 0.016, source: "team", canSave: true });
    expect((await runnerRate("org_b", db)).source).toBe("default");
    await saveRunnerRate("org_a", null, "user_1", db);
    expect((await runnerRate("org_a", db)).source).toBe("default");
    await db.close();
  });

  it("before migration 0006, reads the default and refuses to save", async () => {
    const db = await openPglite();
    expect(await runnerRate("org_a", db)).toEqual({ usdPerMinute: DEFAULT_USD_PER_MINUTE, source: "default", canSave: false });
    await expect(saveRunnerRate("org_a", 0.01, "user_1", db)).rejects.toThrow(/migration 0006/);
    await db.close();
  });
});

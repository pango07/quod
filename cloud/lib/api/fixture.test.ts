import { describe, expect, it } from "vitest";
import { receiptFor } from "../trust";
import { FixtureCloudApi } from "./fixture";
import { resolveApiMode } from "./mode";

const api = new FixtureCloudApi(undefined, 0);

describe("fixture adapter", () => {
  it("adds up: per-day and per-repo minutes match the totals for every range", async () => {
    for (const range of ["7d", "30d", "90d"] as const) {
      const ov = await api.getOverview("quodci", range);
      expect(ov.days.reduce((a, d) => a + d.replayedMinutes, 0)).toBe(ov.totals.savedMinutes);
      expect(ov.repos.reduce((a, r) => a + r.savedMinutes, 0)).toBe(ov.totals.savedMinutes);
      expect(ov.days).toHaveLength(Number(range.slice(0, -1)));
    }
  });

  it("serves run #1842 as the design shows it", async () => {
    const run = await api.getRun("quodci", "quod", 1842);
    expect(run?.tookSeconds).toBe(62);
    expect(run?.wouldHaveTakenSeconds).toBe(400);
    const test = run?.jobs[0].steps.find((s) => s.name === "cargo test --workspace");
    expect(receiptFor(test!).why).toContain("crates/quod-hooks/src/job.rs");
  });

  it("links every step receipt in every run to a step that exists", async () => {
    for (const repo of await api.listRepos("quodci")) {
      const { runs } = await api.listRuns("quodci", { repo: repo.slug });
      for (const r of runs) {
        const run = await api.getRun("quodci", repo.slug, r.number);
        for (const step of run!.jobs.flatMap((j) => j.steps)) {
          const key = receiptFor(step).stepKey;
          if (key) expect(await api.getStepTrust("quodci", repo.slug, key), `${repo.slug} #${r.number} ${key}`).not.toBeNull();
        }
      }
    }
  });

  it("gives a team with no repositories an empty overview, and unknown repos nothing", async () => {
    const ov = await api.getOverview("jvaught", "7d");
    expect(ov.repos).toEqual([]);
    expect(await api.getRepo("quodci", "nope")).toBeNull();
    expect(await api.getRun("quodci", "quod", 9)).toBeNull();
  });

  it("keeps the proofs-needed change", async () => {
    const t = await api.setProofsNeeded("quodci", "payments", "7be2d410", 5);
    expect(t.proofsNeeded).toBe(5);
    expect(t.trust).toMatchObject({ kind: "paused", need: 5 });
  });
});

describe("data source", () => {
  it("serves sample data only when QUOD_API=fixture asks for it", () => {
    for (const NODE_ENV of ["production", "development", "staging", undefined]) {
      expect(resolveApiMode({ NODE_ENV }).kind).toBe("misconfigured");
    }
    expect(resolveApiMode({ NODE_ENV: "development", QUOD_API: "fixture" }).kind).toBe("fixture");
    expect(resolveApiMode({ QUOD_API_URL: "https://api.quodci.com" })).toEqual({ kind: "http", url: "https://api.quodci.com" });
  });
});

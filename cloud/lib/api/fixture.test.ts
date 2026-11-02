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

  it("a real Organization named like the sample team is a real team: its own slug, never sample data", async () => {
    const [team, other] = await api.resolveTeams([
      { orgId: "org_real", name: "quodci" },
      { orgId: "org_other", name: "Acme" },
    ]);
    // Slugs are per viewer: without the demo session's sample team among their teams, it is /quodci.
    expect(team).toEqual({ slug: "quodci", name: "quodci", plan: "Hobby", orgId: "org_real" });
    expect(other).toMatchObject({ slug: "acme", orgId: "org_other" });
    const real = new FixtureCloudApi(undefined, 0, async (slug) => (slug === "quodci" ? "org_real" : null));
    expect(await real.listRepos("quodci")).toEqual([]);
    expect((await real.getOverview("quodci", "7d")).repos).toEqual([]);
    expect(await real.getActivity("quodci")).toMatchObject({ simulated: false, jobs: [] });
    expect(await real.listRunners("quodci")).toEqual([]);
  });

  it("the demo session keeps its sample team; a real org of the same name next to it gets the next slug", async () => {
    const teams = await api.resolveTeams([
      { orgId: "org_real", name: "quodci" },
      { orgId: "org_demo_quodci", name: "quodci" },
    ]);
    expect(teams.map((t) => [t.slug, t.orgId])).toEqual([
      ["quodci-2", "org_real"],
      ["quodci", "org_demo_quodci"],
    ]);
    expect((await api.getActivity("quodci")).simulated).toBe(process.env.QUOD_SIMULATE !== "0");
  });

  it("a slug never takes an app path", async () => {
    const [t] = await api.resolveTeams([{ orgId: "org_x", name: "Sign In" }]);
    expect(t.slug).toBe("sign-in-2");
  });

  it("keeps a team's choices by org id, not by slug", async () => {
    let org = "org_a";
    const f = new FixtureCloudApi(undefined, 0, async () => org);
    await f.saveConnectChoice("acme", { store: "self", host: "own" });
    expect((await f.getConnectSetup("acme")).saved).toEqual({ store: "self", host: "own" });
    org = "org_b"; // another org now has this slug (another viewer)
    expect((await f.getConnectSetup("acme")).saved).toBeNull();
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
  it("serves sample data only when QUOD_API=fixture asks for it; unset is no Quod server, never sample data", () => {
    for (const NODE_ENV of ["production", "development", "staging", undefined]) {
      expect(resolveApiMode({ NODE_ENV }).kind).toBe("none");
    }
    expect(resolveApiMode({ NODE_ENV: "development", QUOD_API: "fixture" }).kind).toBe("fixture");
    expect(resolveApiMode({ QUOD_API: "http", QUOD_SERVER_URL: "https://api.quodci.com/" })).toEqual({ kind: "http", url: "https://api.quodci.com" });
  });

  it("refuses an http setup it cannot use safely", () => {
    const bad = (env: Record<string, string>) => resolveApiMode(env).kind;
    expect(bad({ QUOD_API: "http" })).toBe("misconfigured");
    expect(bad({ QUOD_API: "http", QUOD_SERVER_URL: "not a url" })).toBe("misconfigured");
    expect(bad({ QUOD_API: "http", QUOD_SERVER_URL: "http://quod.example.com" })).toBe("misconfigured");
    expect(bad({ QUOD_API: "http", QUOD_SERVER_URL: "https://user:pw@quod.example.com" })).toBe("misconfigured");
    expect(bad({ QUOD_API: "http", QUOD_SERVER_URL: "https://quod.example.com", QUOD_SERVER_TEAMS: "{" })).toBe("misconfigured");
    expect(bad({ QUOD_API: "http", QUOD_SERVER_URL: "https://quod.example.com", QUOD_SERVER_TEAMS: '{"org_1":{"team":"acme","token":"short"}}' })).toBe("misconfigured");
    expect(bad({ QUOD_API: "htttp" })).toBe("misconfigured");
    // The stub's old variable name is refused rather than silently ignored.
    expect(bad({ QUOD_API_URL: "https://api.quodci.com" })).toBe("misconfigured");
    // Plain http only on the machine or a private network.
    expect(resolveApiMode({ QUOD_API: "http", QUOD_SERVER_URL: "http://localhost:8787" })).toEqual({ kind: "http", url: "http://localhost:8787" });
    expect(bad({ QUOD_API: "http", QUOD_SERVER_URL: "http://quod-server.railway.internal:8080" })).toBe("http");
    expect(bad({ QUOD_API: "http", QUOD_SERVER_URL: "https://quod.example.com", QUOD_SERVER_TEAMS: '{"org_1":{"team":"acme","token":"0123456789abcdef"}}' })).toBe("http");
  });
});

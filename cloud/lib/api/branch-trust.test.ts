import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { HttpCloudApi } from "./http";
import { trustContradictions, trustLabel } from "../trust";

// The "proving 0 of 1" bug: a step recorded once, under proofs 1, on a branch
// that is not a writer. Its run's receipt said "proof 1 of 1"; the Trust list
// said "Proving 0 of 1". Fixtures: contract/branch-only (README there).

const read = (dir: string, f: string) => readFileSync(fileURLToPath(new URL(`./contract/${dir}/${f}`, import.meta.url)), "utf8");

function server(stepsFile: string) {
  return (async (input: URL | RequestInfo) => {
    const path = new URL(String(input)).pathname;
    const body = path.endsWith("/steps") ? read("branch-only", stepsFile) : path.endsWith("/runs/12") ? read("branch-only", "run-12.json") : null;
    return new Response(body ?? '{"error":"not_found","message":"no"}', {
      status: body ? 200 : 404,
      headers: { "content-type": "application/json", "quod-protocol": "quod.v1" },
    });
  }) as typeof fetch;
}

const adapter = (f: typeof fetch) =>
  new HttpCloudApi({ baseUrl: "https://quod.example.com", credentials: async () => ({ team: "acme", token: "t", kind: "static" }), fetch: f, sleep: async () => {} });

describe("a step recorded only on a branch", () => {
  it("the v3 server's list contradicts the run's receipt (the bug, caught)", async () => {
    const api = adapter(server("steps-v3.json"));
    const [steps, run] = await Promise.all([api.listSteps("acme"), api.getRun("acme", "app", 12)]);
    expect(trustLabel(steps[0].trust)).toBe("Proving 0 of 1");
    expect(trustContradictions(steps, [run!])).toEqual([steps[0].key]);
  });

  it("with the fix, the list says it replays, and on which branch", async () => {
    const api = adapter(server("steps.json"));
    const [steps, run] = await Promise.all([api.listSteps("acme"), api.getRun("acme", "app", 12)]);
    expect(trustContradictions(steps, [run!])).toEqual([]);
    expect(steps[0]).toMatchObject({ trust: { kind: "replaying" }, trustRef: "exp/x" });
  });

  it("the recorded main-branch contract has no contradiction either", () => {
    const steps = JSON.parse(read("quod-server", "steps.json")).steps;
    const runs = ["run-1.json", "run-3.json"].map((f) => JSON.parse(read("quod-server", f)));
    expect(trustContradictions(steps, runs)).toEqual([]);
  });
});

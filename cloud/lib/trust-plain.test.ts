import { describe, expect, it } from "vitest";
import type { RunDetail, RunStep, StepSummary } from "./api/types";
import { formatAgo } from "./format";
import { groupSteps, lastResult, plainReceiptFor, plainState, recentByKey, skippedText } from "./trust-plain";
import { pickUi } from "./ui-version";

const K = (n: number) => String(n).repeat(64).slice(0, 64);

const summary = (key: string, over: Partial<StepSummary> = {}): StepSummary => ({
  repoSlug: "quod",
  key,
  job: "cloud",
  command: "npx vitest run --configLoader runner",
  trust: { kind: "proving", have: 0, need: 1 },
  replayRate: null,
  ranSeconds: 11.9,
  replaySeconds: null,
  savedMinutes: 0,
  ...over,
});

const runOf = (number: number, at: string, steps: RunStep[]): RunDetail => ({
  repoSlug: "quod",
  number,
  title: "quod-cloud",
  branch: "exp/cloud-dogfood",
  sha: "0".repeat(40),
  actor: "octocat",
  event: "push",
  runner: "",
  status: "passed",
  startedAt: at,
  tookSeconds: 20,
  wouldHaveTakenSeconds: 20,
  stepsReplayed: 0,
  stepsTotal: steps.length,
  spotChecks: 0,
  spotChecksIdentical: 0,
  githubUrl: "https://github.com/pango07/quod/actions/runs/1",
  jobs: [{ name: "cloud (through quod)", steps }],
});

describe("plain states", () => {
  it("says what a state means in words, never a ratio", () => {
    expect(plainState({ kind: "proving", have: 0, need: 1 }).label).toBe("Learning: needs 1 good run");
    expect(plainState({ kind: "proving", have: 1, need: 3 }).label).toBe("Learning: needs 2 good runs");
    expect(plainState({ kind: "proving", have: 1, need: 1 }).label).toBe("Ready to skip");
    expect(plainState({ kind: "replaying" }, null, null).label).toBe("Ready to skip");
    expect(plainState({ kind: "replaying" }, { skips: 3, checks: 2, network: false }).label).toBe("Skipping: checked 2 times");
    expect(plainState({ kind: "paused", consistent: 0, need: 2 }).label).toBe("Paused: a re-check found different output");
    expect(plainState({ kind: "proving", have: 0, need: 1 }, { skips: 0, checks: 0, network: true }).label).toBe("Always runs: uses the network");
    for (const t of [{ kind: "replaying" }, { kind: "watching", wouldHaveReplayed: 2 }] as const) expect(plainState(t).help.length).toBeGreaterThan(40);
  });

  it("the switch: query, then cookie, then the build default", () => {
    expect(pickUi(undefined, undefined, undefined)).toBe("classic");
    expect(pickUi(undefined, undefined, "1")).toBe("plain");
    expect(pickUi("classic", "plain", "1")).toBe("classic");
    expect(pickUi(undefined, "plain", undefined)).toBe("plain");
    expect(pickUi("new", undefined, undefined)).toBe("plain");
    expect(pickUi("nonsense", undefined, undefined)).toBe("classic");
  });
});

describe("grouping by job and command", () => {
  // The live page: one command, a new key per dogfood run (Quod rebuilt each time), and one key without a command.
  const steps = [summary(K(1)), summary(K(2), { trust: { kind: "replaying" }, replaySeconds: 0.2 }), summary(K(3)), summary(K(4), { command: "", job: "" })];
  const runs = [
    runOf(10, "2026-11-02T18:00:00Z", [{ name: "npm test (through quod)", outcome: "ran", seconds: 11.9, why: { kind: "first-run", stepKey: K(1), recordedInputs: 900, proof: { have: 1, need: 1 } } }]),
    runOf(12, "2026-11-02T20:00:00Z", [
      { name: "npm test (through quod)", outcome: "replayed", seconds: 0.2, why: { kind: "replayed", stepKey: K(2), sourceRun: 11, inputsMatched: 900, proof: { have: 1, need: 1 }, divergences: 0, savedSeconds: 11.7 } },
      { name: "shadow warm-up", index: 7, outcome: "ran", seconds: 30, why: { kind: "first-run", stepKey: K(4), recordedInputs: 10, proof: { have: 1, need: 1 } } },
    ]),
  ];
  const groups = groupSteps(steps, recentByKey(runs));

  it("folds the versions of one command under the newest", () => {
    const vitest = groups.find((g) => g.title.startsWith("npx vitest"))!;
    expect(vitest.current.step.key).toBe(K(2));
    expect(vitest.earlier.map((v) => v.step.key)).toEqual([K(1), K(3)]);
    expect(vitest.reason).toMatch(/Quod itself was updated/);
  });

  it("names a step without a command by GitHub's job and step", () => {
    const g = groups.find((x) => x.current.step.key === K(4))!;
    expect(g).toMatchObject({ untitled: true, title: "cloud (through quod) › shadow warm-up" });
    expect(groupSteps([summary(K(5), { command: "" })], new Map())[0].title).toBe("Unnamed step (#1)");
  });

  it("columns: last result, skipped, never n/a", () => {
    const vitest = groups.find((g) => g.title.startsWith("npx vitest"))!;
    expect(lastResult(vitest.current, "2026-11-02T20:04:00Z", formatAgo)).toBe("Skipped in 0.20s · saved 11.7s · 4m ago");
    expect(skippedText(vitest.current)).toBe("Skipped 1×");
    expect(lastResult(vitest.earlier[0], "2026-11-02T20:04:00Z", formatAgo)).toBe("Ran in 11.9s · 2h ago");
    expect(skippedText(vitest.earlier[1])).toBe("Not skipped yet");
    expect(lastResult(vitest.earlier[1], "2026-11-02T20:04:00Z", formatAgo)).toBe("Runs in 11.9s");
    expect(vitest.current.state.label).toBe("Skipping");
  });
});

describe("a run's receipts, plain", () => {
  it("one recorded run under proofs 1 is ready to skip, in words", () => {
    const r = plainReceiptFor({ name: "npm test", outcome: "ran", seconds: 11.9, why: { kind: "first-run", stepKey: K(1), recordedInputs: 900, proof: { have: 1, need: 1 } } });
    expect(r.receipt).toContain("ready to skip next time");
    expect(r.receipt).not.toMatch(/proof \d of \d/);
    const s = plainReceiptFor({ name: "npm test", outcome: "replayed", seconds: 0.2, why: { kind: "replayed", stepKey: K(1), sourceRun: 11, inputsMatched: 900, proof: { have: 1, need: 1 }, divergences: 0, savedSeconds: 11.7 } });
    expect(s.state).toBe("Skipped");
    expect(s.receipt.split("\n")[0]).toBe("QUOD  SKIPPED · result from run #11");
  });
});

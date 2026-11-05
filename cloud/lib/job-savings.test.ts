import { describe, expect, it } from "vitest";
import { savingsSummary, skippedExecution } from "./job-savings";
import type { JobSavings } from "./api/types";
import type { TestRunLite } from "./savings";

const window: JobSavings = { jobs:12, baseline:"observed", runnerSeconds:1173, baselineRunnerSeconds:1905, billedMinutes:25, baselineBilledMinutes:36, costUsd:0, baselineCostUsd:0 };
describe("whole-job savings", () => {
  it("keeps runner-time, per-job billing, and free-runner cost distinct", () => {
    const s = savingsSummary(window)!;
    expect(s.seconds).toBe(732);
    expect(s.timeShare).toBeCloseTo(0.38425, 4);
    expect(s.minutes).toBe(11);
    expect(s.billingShare).toBeCloseTo(0.30556, 4);
    expect(s.costUsd).toBe(0);
  });
  it("never prices skipped steps or invents a missing runner rate", () => {
    expect(savingsSummary(undefined)).toBeNull();
    expect(savingsSummary({...window, costUsd:null})!.costUsd).toBeNull();
  });
  it("preserves regressions and distinguishes an estimated baseline", () => {
    const s = savingsSummary({...window, runnerSeconds:2000, billedMinutes:40, costUsd:1, baselineCostUsd:0.8, baseline:"estimated"})!;
    expect(s.seconds).toBe(-95);
    expect(s.minutes).toBe(-4);
    expect(s.costUsd).toBeCloseTo(-0.2);
    expect(s.estimated).toBe(true);
  });
  it("rejects invalid or empty comparisons", () => {
    expect(savingsSummary({...window, jobs:0})).toBeNull();
    expect(savingsSummary({...window, runnerSeconds:NaN})).toBeNull();
  });
});

describe("avoided test execution", () => {
  const run: TestRunLite = {
    id:"check", at:"2026-11-05T12:00:00Z", repoSlug:"app", repository:"team/app",
    suite:".", kind:"critical", event:"push", branch:"main", runId:1, sha:"abc",
    files:{total:10, ran:2, replayed:8}, timing:{wallMs:10_000, estPlainMs:70_000, savedMs:60_000},
  };
  const from = Date.parse("2026-11-05T00:00:00Z");
  const to = Date.parse("2026-11-06T00:00:00Z");
  it("counts each in-range check once and excludes legacy background recordings", () => {
    expect(skippedExecution([
      run, run,
      {...run, id:"legacy", kind:"shadow"},
      {...run, id:"inline", kind:"shadow", sha:"def"},
      {...run, id:"old", at:"2026-11-01T00:00:00Z"},
      {...run, id:"future", at:"2026-11-07T00:00:00Z"},
    ], from, to)).toEqual({minutes:2, measured:2, runs:2});
  });
  it("keeps missing measurements distinct from measured zero and reports coverage", () => {
    const unknown = {...run, timing:{...run.timing, savedMs:null}};
    expect(skippedExecution(null, from, to)).toBeNull();
    expect(skippedExecution([], from, to)).toBeNull();
    expect(skippedExecution([unknown], from, to)).toEqual({minutes:null, measured:0, runs:1});
    expect(skippedExecution([unknown, {...run, id:"zero", timing:{...run.timing, savedMs:0}}], from, to))
      .toEqual({minutes:0, measured:1, runs:2});
  });
});

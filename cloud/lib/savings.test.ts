import { describe, expect, it } from "vitest";
import { savingsSummary } from "./savings";
import type { JobSavings } from "./api/types";

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

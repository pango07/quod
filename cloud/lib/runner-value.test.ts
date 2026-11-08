import { describe, expect, it } from "vitest";
import { formatRunnerValue, modeledRunnerValue, type RunnerValueRate } from "./runner-value";

describe("runner time valued at the configured rate", () => {
  it("prices the complete measured seconds, including a regression, without rounded minutes", () => {
    expect(modeledRunnerValue(175, 0.006)).toBeCloseTo(0.0175, 12);
    expect(modeledRunnerValue(100, 0.006)).toBeCloseTo(0.01, 12);
    expect(modeledRunnerValue(100 - 175, 0.006)).toBeCloseTo(-0.0075, 12);
    expect(modeledRunnerValue(100 - 175, 0.016)).toBeCloseTo(-0.02, 12);
  });

  it("values time within and across billing-minute boundaries fractionally", () => {
    // Both 61s and 119s round to two provider minutes, but their time still has value.
    expect(modeledRunnerValue(119 - 61, 0.006)).toBeCloseTo(0.0058, 12);
    expect(modeledRunnerValue(59 - 61, 0.006)).toBeCloseTo(-0.0002, 12);
    expect(modeledRunnerValue(0.5, 0.006)).toBeCloseTo(0.00005, 12);
    expect(modeledRunnerValue(0, 0.006)).toBe(0);
  });

  it("uses the supplied rate regardless of whether its source is a team or default", () => {
    const rates: RunnerValueRate[] = [
      { usdPerMinute: 0.006, source: "team" },
      { usdPerMinute: 0.006, source: "default" },
    ];
    // Actual public-runner charges are deliberately not an input to this calculation.
    for (const rate of rates) expect(modeledRunnerValue(75, rate.usdPerMinute)).toBeCloseTo(0.0075, 12);
    expect(modeledRunnerValue(75, 0)).toBe(0);
  });

  it("keeps missing, invalid and overflowing measurements distinct from zero", () => {
    for (const seconds of [null, NaN, Infinity, -Infinity]) {
      expect(modeledRunnerValue(seconds, 0.006)).toBeNull();
    }
    for (const rate of [null, undefined, NaN, Infinity, -Infinity, -0.001]) {
      expect(modeledRunnerValue(75, rate)).toBeNull();
    }
    expect(modeledRunnerValue(Number.MAX_VALUE, Number.MAX_VALUE)).toBeNull();
    expect(modeledRunnerValue(-Number.MAX_VALUE, Number.MAX_VALUE)).toBeNull();
  });
});

describe("runner value display", () => {
  it("shows sub-cent amounts precisely and preserves negative differences", () => {
    expect(formatRunnerValue(0.0175)).toBe("$0.0175");
    expect(formatRunnerValue(0.01)).toBe("$0.0100");
    expect(formatRunnerValue(-0.0075)).toBe("−$0.0075");
    expect(formatRunnerValue(-0.0002)).toBe("−$0.0002");
    expect(formatRunnerValue(1)).toBe("$1.00");
    expect(formatRunnerValue(12.476)).toBe("$12.48");
    expect(formatRunnerValue(-1234.5)).toBe("−$1,234.50");
  });

  it("distinguishes exact zero, very small nonzero values and invalid numbers", () => {
    expect(formatRunnerValue(0)).toBe("$0.00");
    expect(formatRunnerValue(-0)).toBe("$0.00");
    expect(formatRunnerValue(0.0001)).toBe("$0.0001");
    expect(formatRunnerValue(0.00005)).toBe("<$0.0001");
    expect(formatRunnerValue(-0.00005)).toBe("−<$0.0001");
    for (const value of [NaN, Infinity, -Infinity]) expect(formatRunnerValue(value)).toBe("n/a");
  });
});

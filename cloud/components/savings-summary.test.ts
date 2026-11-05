import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { TeamTotals } from "@/lib/api/types";
import { SavingsSummary } from "./savings-summary";

const totals: TeamTotals = {range:"7d", savedMinutes:100, stepsReplayed:60, stepsTotal:100,
  costSavedUsd:0.8, usdPerMinute:0.008, wrongReplays:0, spotChecks:0, spotChecksIdentical:0};
const ci = {medianSeconds:20, measuredRuns:12, sample:false, runs:[]};
describe("overview savings evidence", () => {
  it("does not present legacy skipped-step estimates as net money or workflow time saved", () => {
    const html = renderToStaticMarkup(createElement(SavingsSummary, {totals, ci}));
    expect(html).toContain("Execution avoided");
    expect(html).toContain("before overhead");
    expect(html).toContain("20s");
    expect(html).toContain("Measuring");
    expect(html).not.toContain("$0.80");
    expect(html).not.toContain("minutes you did not wait for");
  });
  it("shows zero dollars for known free runners and preserves the time saving", () => {
    const html = renderToStaticMarkup(createElement(SavingsSummary, {ci, totals:{...totals,
      jobSavings:{jobs:12,baseline:"observed",runnerSeconds:1173,baselineRunnerSeconds:1905,billedMinutes:25,baselineBilledMinutes:36,costUsd:0,baselineCostUsd:0}}}));
    expect(html).toContain("12m 12s");
    expect(html).toContain("38% less");
    expect(html).toContain("$0.00");
    expect(html).not.toContain("Measuring");
  });
  it("keeps additional time/cost visible instead of clamping it to zero", () => {
    const html = renderToStaticMarkup(createElement(SavingsSummary, {ci, totals:{...totals,
      jobSavings:{jobs:1,baseline:"observed",runnerSeconds:70,baselineRunnerSeconds:50,billedMinutes:2,baselineBilledMinutes:1,costUsd:0.02,baselineCostUsd:0.01}}}));
    expect(html).toContain("Extra runner time");
    expect(html).toContain("40% more");
    expect(html).toContain("Est. extra runner cost");
  });
});

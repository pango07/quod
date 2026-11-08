import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ObservedCiComparisons } from "./observed-ci-comparisons";
import type { CiComparison } from "@/lib/ci-comparisons";
import type { RunnerValueRate } from "@/lib/runner-value";

const pair: CiComparison = {
  runId: 100, attempt: 1, sha: "a".repeat(40), finishedAt: "2026-11-08T10:05:00Z", runUrl: "https://github.com/pango07/quod/actions/runs/100/attempts/1",
  candidate: { seconds: 70, url: "https://github.com/pango07/quod/actions/runs/100/job/200" },
  baseline: { seconds: 50, url: "https://github.com/pango07/quod/actions/runs/100/job/201" }, savedSeconds: -20, cost: "unknown",
};
const render = (pairs: CiComparison[], valuationRate:RunnerValueRate = {usdPerMinute:0.006, source:"team"}) => renderToStaticMarkup(createElement(ObservedCiComparisons, { data: { visible: true, available: true, inspectedRuns: 1, pairs }, valuationRate }));

describe("observed CI comparison display", () => {
  it("keeps extra runner time visible, links sources and leaves unsupported cost unknown", () => {
    const html = render([pair]);
    expect(html).toContain("20s more");
    expect(html).toContain("70s");
    expect(html).toContain("1m 10s");
    expect(html).toContain("50s");
    expect(html).toContain("Measuring");
    expect(html).toContain("$0.0020 extra");
    expect(html).toContain("$0.006 per minute");
    expect(html).toContain(pair.runUrl);
    expect(html).toContain(pair.candidate.url);
    expect(html).toContain(`/commit/${pair.sha}`);
    expect(html).not.toContain("$0.00 saved");
  });
  it("prices extra time even when compute is free, and keeps the two values separate", () => {
    const html = render([{ ...pair, cost: "public-standard-free" }]);
    expect(html).toContain("$0.00 each");
    expect(html).toContain("$0.0020 extra");
    expect(html).toContain("Modeled time value");
    expect(html).toContain("Storage and Quod subscription costs are excluded");
  });
  it("uses the saved team rate for the observed 75-second regression and for savings", () => {
    const actual = { ...pair, candidate:{...pair.candidate, seconds:175}, baseline:{...pair.baseline, seconds:100}, savedSeconds:-75, cost:"public-standard-free" as const };
    expect(render([actual])).toContain("$0.0075 extra");
    expect(render([actual], {usdPerMinute:0.016,source:"team"})).toContain("$0.0200 extra");
    expect(render([{...actual, candidate:actual.baseline, baseline:actual.candidate, savedSeconds:75}])).toContain("$0.0075 saved");
    expect(render([actual], {usdPerMinute:0.006,source:"default"})).toContain("default valuation rate");
  });
  it("leaves valuation unknown when the saved rate could not be loaded", () => {
    const html = renderToStaticMarkup(createElement(ObservedCiComparisons, {data:{visible:true,available:true,inspectedRuns:1,pairs:[{...pair,cost:"public-standard-free"}]}}));
    expect(html).toContain("Measuring");
    expect(html).toContain("Runner price is unavailable");
    expect(html).toContain("$0.00 each");
    expect(html).not.toContain("$0.0020");
  });
  it("waits for an observed pair and hides unrelated teams", () => {
    expect(render([])).toContain("Waiting for a completed comparison");
    expect(render([])).not.toContain("$0.00 saved");
    expect(renderToStaticMarkup(createElement(ObservedCiComparisons, { data: { visible: false, available: false, inspectedRuns: 0, pairs: [] } }))).toBe("");
  });
});

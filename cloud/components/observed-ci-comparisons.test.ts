import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ObservedCiComparisons } from "./observed-ci-comparisons";
import type { CiComparison } from "@/lib/ci-comparisons";

const pair: CiComparison = {
  runId: 100, attempt: 1, sha: "a".repeat(40), finishedAt: "2026-11-08T10:05:00Z", runUrl: "https://github.com/pango07/quod/actions/runs/100/attempts/1",
  candidate: { seconds: 70, url: "https://github.com/pango07/quod/actions/runs/100/job/200" },
  baseline: { seconds: 50, url: "https://github.com/pango07/quod/actions/runs/100/job/201" }, savedSeconds: -20, cost: "unknown",
};
const render = (pairs: CiComparison[]) => renderToStaticMarkup(createElement(ObservedCiComparisons, { data: { visible: true, available: true, inspectedRuns: 1, pairs } }));

describe("observed CI comparison display", () => {
  it("keeps extra runner time visible, links sources and leaves unsupported cost unknown", () => {
    const html = render([pair]);
    expect(html).toContain("20s more");
    expect(html).toContain("70s");
    expect(html).toContain("1m 10s");
    expect(html).toContain("50s");
    expect(html).toContain("Measuring");
    expect(html).toContain(pair.runUrl);
    expect(html).toContain(pair.candidate.url);
    expect(html).toContain(`/commit/${pair.sha}`);
    expect(html).not.toContain("$0.00 saved");
  });
  it("shows zero compute savings for known-free runners, not a modeled rate", () => {
    const html = render([{ ...pair, cost: "public-standard-free" }]);
    expect(html).toContain("$0.00 each · $0.00 saved");
    expect(html).toContain("Storage and Quod subscription costs are excluded");
  });
  it("waits for an observed pair and hides unrelated teams", () => {
    expect(render([])).toContain("Waiting for a completed comparison");
    expect(render([])).not.toContain("$0.00 saved");
    expect(renderToStaticMarkup(createElement(ObservedCiComparisons, { data: { visible: false, available: false, inspectedRuns: 0, pairs: [] } }))).toBe("");
  });
});

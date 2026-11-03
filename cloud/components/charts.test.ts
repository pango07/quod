import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { TestRunSummary } from "@/lib/api/types";
import { lastUtcDays, ranMs, testMinutesPerDay } from "@/lib/test-runs";
import { MinutesChart } from "./charts";

// The Overview's per-day chart fed from test check runs.

const NOW = Date.parse("2026-11-02T12:00:00Z");
const run = (at: string, kind: TestRunSummary["kind"], timing: Partial<TestRunSummary["timing"]>) => ({
  at,
  kind,
  timing: { wallMs: null, selectMs: null, estPlainMs: null, savedMs: null, ...timing },
});

// Two days, checks and background recordings mixed; one check without wallMs.
const runs = [
  run("2026-11-02T09:00:00Z", "critical", { wallMs: 21_000, savedMs: 79_000, estPlainMs: 119_000 }),
  run("2026-11-02T09:02:00Z", "shadow", { wallMs: 30_000, savedMs: 0, estPlainMs: 119_000 }),
  run("2026-11-02T10:00:00Z", "critical", { wallMs: null, savedMs: 38_000, estPlainMs: 39_000 }),
  run("2026-11-01T23:59:00Z", "critical", { wallMs: 6_000, savedMs: 12_000 }),
  run("2026-11-01T08:00:00Z", "shadow", { wallMs: 90_000, savedMs: 50_000 }),
  // Outside the chart's days: left out.
  run("2026-10-20T08:00:00Z", "critical", { wallMs: 60_000, savedMs: 60_000 }),
];

describe("test time per day", () => {
  it("the days: the last n UTC days, oldest first", () => {
    expect(lastUtcDays(3, NOW)).toEqual(["2026-10-31", "2026-11-01", "2026-11-02"]);
  });

  it("ran: wall time, or (not reported) the ran files' recorded time", () => {
    expect(ranMs(run("x", "critical", { wallMs: 21_000, estPlainMs: 119_000, savedMs: 79_000 }))).toBe(21_000);
    expect(ranMs(run("x", "critical", { wallMs: null, estPlainMs: 39_000, savedMs: 38_000 }))).toBe(1_000);
    expect(ranMs(run("x", "critical", {}))).toBe(0);
  });

  it("checks only, per UTC day: saved = sum of savedMs, ran = sum of ranMs; shadow rows change nothing", () => {
    const days = testMinutesPerDay(runs, lastUtcDays(3, NOW));
    expect(days).toEqual([
      { day: "2026-10-31", replayedMinutes: 0, ranMinutes: 0 },
      { day: "2026-11-01", replayedMinutes: 12_000 / 60_000, ranMinutes: 6_000 / 60_000 },
      { day: "2026-11-02", replayedMinutes: (79_000 + 38_000) / 60_000, ranMinutes: (21_000 + 1_000) / 60_000 },
    ]);
    expect(testMinutesPerDay(runs.filter((r) => r.kind === "critical"), lastUtcDays(3, NOW))).toEqual(days);
  });

  it("today is not all 'ran': the bars and the table show the same values, saved marked ≈", () => {
    const html = renderToStaticMarkup(createElement(MinutesChart, { days: testMinutesPerDay(runs, lastUtcDays(3, NOW)), from: "3 days ago", source: "tests" }));
    const text = html.replace(/<[^>]+>/g, " ").replace(/\s+/g, " ");
    // Tooltip and table, today: 117 s saved (≈ 2 min), 22 s ran.
    expect(text).toContain("Saved ≈ 2 min, ran 22 s");
    expect(text).toMatch(/≈ Saved Ran Nov 2 ≈ 2 min 22 s Nov 1 ≈ 12 s 6 s Oct 31 0 s 0 s/);
    expect(html).toContain('class="seg-blue" style="height:84.17%"');
  });
});

import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { TestStatsCard } from "@/components/test-runs";
import { sampleTestRuns, sampleTestStats, sampleTestSuites, summaryOnly } from "@/lib/api/fixture-tests";
import type { TestRunDetail, TestRunSummary, TestSuite } from "@/lib/api/types";
import { formatSaved, formatSavedMs, showSpotChecks, spotChecksWithinWeek } from "./test-runs";
import { barSegments, lookReason, needLookCount, recordDiffers, savedWithinWeek, suiteRowSummary, trustModel } from "./test-suites";

// The audit's fixes: sub-second times, "≈" on estimates, spot-check windows,
// "recording differed", reasons as sent, and no empty spot-check section.

const NOW = Date.parse("2026-11-02T12:00:00Z");
const text = (html: string) => html.replace(/<[^>]+>/g, " ").replace(/\s+/g, " ");

describe("times", () => {
  it("under a second has one decimal, never 0 s for a real time", () => {
    expect(formatSavedMs(400)).toBe("0.4 s");
    expect(formatSavedMs(960)).toBe("1 s");
    expect(formatSavedMs(50)).toBe("0.1 s");
    expect(formatSavedMs(30)).toBe("<0.1 s");
    expect(formatSavedMs(0)).toBe("0 s");
    expect(formatSavedMs(5_700)).toBe("5.7 s");
  });

  it("saved is an estimate: ≈, except none at all or unknown", () => {
    expect(formatSaved(5_700)).toBe("≈ 5.7 s");
    expect(formatSaved(400)).toBe("≈ 0.4 s");
    expect(formatSaved(79_240)).toBe("≈ 79 s");
    expect(formatSaved(0)).toBe("0 s");
    expect(formatSaved(null)).toBe("n/a");
  });
});

describe("spot-checks", () => {
  const at = (h: number) => new Date(NOW - h * 3_600_000).toISOString();
  const r = (kind: TestRunSummary["kind"], h: number, checked: number, mismatched = 0) => ({ kind, at: at(h), audits: { checked, mismatched } });

  it("the last 7 days, from the background recordings only (a check carries its shadow's: counted once)", () => {
    expect(spotChecksWithinWeek([r("shadow", 1, 4), r("critical", 1, 4), r("shadow", 30, 3, 1), r("shadow", 24 * 8, 9)], NOW)).toEqual({ checked: 7, mismatched: 1 });
    expect(spotChecksWithinWeek([], NOW)).toEqual({ checked: 0, mismatched: 0 });
  });

  it("the Overview card says which window it counts", () => {
    const stats = sampleTestStats(NOW);
    const week = text(renderToStaticMarkup(createElement(TestStatsCard, { team: "t", stats, spotChecksWeek: { checked: 6, mismatched: 0 } })));
    expect(week).toContain("Spot-checks passed, last 7 days 6 of 6");
    const all = text(renderToStaticMarkup(createElement(TestStatsCard, { team: "t", stats })));
    expect(all).toContain("Spot-checks passed, all time");
    expect(all).toMatch(/Time saved this week ≈ /);
  });

  it("Trust counts the last 7 days from the runs it already reads", () => {
    const m = trustModel({ team: "t", suites: sampleTestSuites(NOW), runs: sampleTestRuns(NOW).map(summaryOnly), stats: sampleTestStats(NOW), now: NOW, ago: () => "" });
    expect(m.summary.spotChecksWindow).toBe("week");
    expect(m.summary.spotChecksPassed).toBe(spotChecksWithinWeek(sampleTestRuns(NOW), NOW).checked);
    const noRuns = trustModel({ team: "t", suites: sampleTestSuites(NOW), runs: [], stats: sampleTestStats(NOW), now: NOW, ago: () => "" });
    expect(noRuns.summary.spotChecksWindow).toBe("all");
  });

  it("Trust's week: quod-server's stats miss runs recorded inline, so with any among the runs the week is the runs' own", () => {
    const runs = sampleTestRuns(NOW).map(summaryOnly);
    const stats = sampleTestStats(NOW);
    const m = trustModel({ team: "t", suites: sampleTestSuites(NOW), runs, stats, now: NOW, ago: () => "" });
    const own = savedWithinWeek(runs, NOW);
    expect(own.runs).toBeGreaterThan(stats.week.runs);
    expect([m.summary.weekRuns, m.summary.savedWeekMs]).toEqual([own.runs, own.savedMs]);
    // Legacy runs only: the server's week, as before.
    const legacyOnly = runs.filter((r) => r.kind === "critical" || r.id.endsWith("s"));
    expect(trustModel({ team: "t", suites: sampleTestSuites(NOW), runs: legacyOnly, stats, now: NOW, ago: () => "" }).summary.weekRuns).toBe(stats.week.runs);
  });

  it("the Overview card's copy: spot-checks run in the same job, never in the background", () => {
    const html = text(renderToStaticMarkup(createElement(TestStatsCard, { team: "t", stats: sampleTestStats(NOW), compact: true })));
    expect(html).toContain("in the same job");
    expect(html).not.toMatch(/background/i);
  });

  it("a run's page: no spot-check section when there is nothing to say", () => {
    const run = (over: Partial<TestRunDetail>) => ({ ...sampleTestRuns(NOW).find((x) => x.id === "tr_4182c")!, ...over });
    // Replayed nothing: nothing to check.
    expect(showSpotChecks(run({ files: { total: 9, ran: 9, replayed: 0, narrowed: 0 }, auditFiles: [], audits: null, shadow: null }))).toBe(false);
    // Reported, without spot-checks.
    expect(showSpotChecks(run({ auditFiles: [], audits: { checked: 0, mismatched: 0 } }))).toBe(false);
    // Skipped files and no spot-checks: they run in the same job now, so there is nothing to wait for.
    expect(showSpotChecks(run({ auditFiles: [], audits: null, shadow: null }))).toBe(false);
    // Spot-checks: shown.
    expect(showSpotChecks(run({}))).toBe(true);
  });
});

describe("recording differed (totals.recordDiffers)", () => {
  const t = (over: Partial<TestSuite["totals"]>): TestSuite["totals"] => ({ files: 10, learning: 0, ready: 1, skipping: 6, always: 2, paused: 1, savedMs: 0, ...over });

  it("absent or 0: as before", () => {
    expect(recordDiffers(t({}))).toEqual({ n: 0, inAlways: false });
    expect(barSegments(t({})).map((s) => s.state)).toEqual(["ready", "skipping", "always", "paused"]);
    expect(suiteRowSummary(t({}))).toBe("10 files · 6 trusted · 3 need a look");
  });

  it("counted inside always (the states already add up to files): taken out of always, its own segment", () => {
    const x = t({ recordDiffers: 1 });
    expect(recordDiffers(x)).toEqual({ n: 1, inAlways: true });
    expect(barSegments(x).map((s) => [s.state, s.n])).toEqual([
      ["ready", 1],
      ["skipping", 6],
      ["always", 1],
      ["recordDiffers", 1],
      ["paused", 1],
    ]);
    expect(needLookCount(x)).toBe(3);
    expect(suiteRowSummary(x)).toBe("10 files · 6 trusted · 3 need a look (1 where runs disagreed)");
  });

  it("counted apart (files = states + recordDiffers): added, not folded into always", () => {
    const x = t({ files: 12, recordDiffers: 2 });
    expect(recordDiffers(x)).toEqual({ n: 2, inAlways: false });
    expect(barSegments(x).find((s) => s.state === "always")?.n).toBe(2);
    expect(barSegments(x).find((s) => s.state === "recordDiffers")).toEqual({ state: "recordDiffers", n: 2, label: "runs disagreed" });
    expect(suiteRowSummary(x)).toBe("12 files · 6 trusted · 5 need a look (2 where runs disagreed)");
  });

  it("in the summary strip's legend", () => {
    const suites = sampleTestSuites(NOW).map((s, i) => (i === 0 ? { ...s, totals: { ...s.totals, recordDiffers: 1 } } : s));
    const m = trustModel({ team: "t", suites, runs: [], stats: null, now: NOW, ago: () => "" });
    expect(m.summary.bar.map((b) => b.label)).toContain("runs disagreed");
    expect(m.suites[0].summary).toContain("(1 where runs disagreed)");
  });
});

describe("always-runs reasons", () => {
  it("shown as the server sends them", () => {
    expect(lookReason({ state: "always", reason: "reads the per-run variable GITHUB_EVENT_PATH" })).toBe("reads the per-run variable GITHUB_EVENT_PATH");
    expect(lookReason({ state: "always", reason: "uses the network" })).toBe("uses the network");
    expect(lookReason({ state: "always", reason: null })).toBe("Quod runs it every time.");
  });
});

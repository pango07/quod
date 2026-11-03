import { describe, expect, it } from "vitest";
import { sampleTestRuns, sampleTestSuites, summaryOnly } from "@/lib/api/fixture-tests";
import type { TestFile, TestRunSummary, TestSuite } from "@/lib/api/types";
import {
  fileStateLabel,
  filterCounts,
  lastResultText,
  matchesFilter,
  MAX_FILES,
  medianSkippedPct,
  NO_RECORDING,
  runsOfSuite,
  sortFiles,
  stateCounts,
  suiteHeading,
  suiteLedgerText,
  suiteRowSummary,
  suiteTotalsText,
  testFilesLine,
  trustModel,
  trustSections,
  visibleFiles,
} from "./test-suites";

const NOW = Date.parse("2026-11-02T12:00:00Z");
const file = (over: Partial<TestFile>): TestFile => ({ file: "src/a.test.ts", project: "", state: "ready", reason: null, lastResult: "pass", skipped: 0, ran: 1, durationMs: 100, savedMs: 0, ...over });

describe("test file states in words", () => {
  it("one label per state, with the counts and reasons the server sends", () => {
    expect(fileStateLabel(file({ state: "learning", reason: "not recorded yet" })).label).toBe("Learning");
    expect(fileStateLabel(file({ state: "ready" })).label).toBe("Ready to skip");
    expect(fileStateLabel(file({ state: "skipping", skipped: 12 })).label).toBe("Skipping: skipped 12 times");
    expect(fileStateLabel(file({ state: "skipping", skipped: 1 })).label).toBe("Skipping: skipped 1 time");
    expect(fileStateLabel(file({ state: "always", reason: "it failed last time" })).label).toBe("Always runs: it failed last time");
    expect(fileStateLabel(file({ state: "always", reason: null })).label).toBe("Always runs");
    expect(fileStateLabel(file({ state: "paused", reason: "anything" })).label).toBe("Paused: a spot-check differed");
  });

  it("tones match the plain trust pages: paused amber, skipping and ready blue", () => {
    expect(fileStateLabel(file({ state: "paused" })).tone).toBe("amber");
    expect(fileStateLabel(file({ state: "skipping", skipped: 2 })).tone).toBe("blue");
    expect(fileStateLabel(file({ state: "ready" })).tone).toBe("blue");
    expect(fileStateLabel(file({ state: "always" })).tone).toBe("dim");
  });

  it("an unknown state from a newer server is shown as it is, not thrown", () => {
    expect(fileStateLabel(file({ state: "quarantined" as TestFile["state"] })).label).toBe("quarantined");
  });

  it("last results", () => {
    expect([lastResultText("pass"), lastResultText("fail"), lastResultText("skip"), lastResultText(null)]).toEqual(["Passed", "Failed", "Skipped by vitest", "Not recorded yet"]);
  });
});

describe("suites", () => {
  it("heading: repo name and package; the root suite is the repo root", () => {
    expect(suiteHeading({ repository: "acme/class-management", suite: "packages/backend" })).toBe("class-management · packages/backend");
    expect(suiteHeading({ repository: "pango07/ci-bench", suite: "." })).toBe("ci-bench · repo root");
  });

  it("which recording it reads: short sha, branch, when", () => {
    const ago = () => "2 h ago";
    expect(suiteLedgerText({ at: "x", sha: "1d7e5b2c90a4f3e8", branch: "main", scope: "main" }, ago)).toBe("Reads the recording of main at 1d7e5b2, 2 h ago.");
    expect(suiteLedgerText({ at: "x", sha: null, branch: "main", scope: "main" }, ago)).toBe("Reads the latest recording of main, 2 h ago.");
    expect(suiteLedgerText({ at: "x", sha: "a93d0e7c", branch: "dev", scope: "dev" }, ago)).toBe("Reads the recording of dev at a93d0e7, 2 h ago.");
  });

  it("no recording: names a writer branch, never the default branch", () => {
    expect(suiteLedgerText(null, () => "")).toBe("No recording yet: the first run on a writer branch records these files.");
    expect(NO_RECORDING).not.toMatch(/default/i);
  });

  it("totals: zeros left out, in a fixed order", () => {
    expect(stateCounts({ skipping: 152, ready: 9, learning: 0, always: 3, paused: 0 })).toBe("152 skipping, 9 ready, 3 always run");
    expect(suiteTotalsText({ files: 1, skipping: 0, ready: 0, learning: 1, always: 0, paused: 0, savedMs: 0 })).toBe("1 test file: 1 learning.");
    expect(suiteTotalsText({ files: 10, skipping: 10, ready: 0, learning: 0, always: 0, paused: 0, savedMs: 90_000 })).toBe("10 test files: 10 skipping. Saved 2 min.");
  });

  it("row summary: files, skipping, need a look", () => {
    expect(suiteRowSummary({ files: 184, skipping: 170, ready: 10, learning: 1, always: 2, paused: 1, savedMs: 0 })).toBe("184 files · 170 skipping · 3 need a look");
    expect(suiteRowSummary({ files: 9, skipping: 0, ready: 0, learning: 9, always: 0, paused: 0, savedMs: 0 })).toBe("9 files");
    expect(suiteRowSummary({ files: 2, skipping: 1, ready: 0, learning: 0, always: 1, paused: 0, savedMs: 0 })).toBe("2 files · 1 skipping · 1 needs a look");
  });

  it("the Overview line adds every suite up; none: no line", () => {
    expect(testFilesLine(sampleTestSuites(NOW))).toBe("296 test files: 265 skipping, 17 ready, 10 learning, 3 always run, 1 paused");
    expect(testFilesLine([])).toBeNull();
    expect(testFilesLine(null)).toBeNull();
  });

  it("the sample suites' totals match their files, and the files come in the server's order", () => {
    const order = ["paused", "always", "learning", "skipping", "ready"];
    for (const s of sampleTestSuites(NOW)) {
      expect(s.totals.files).toBe(s.files.length);
      expect(s.totals.skipping).toBe(s.files.filter((f) => f.state === "skipping").length);
      const idx = s.files.map((f) => order.indexOf(f.state));
      expect(idx).toEqual([...idx].sort((a, b) => a - b));
    }
  });
});

describe("what Trust shows", () => {
  const suites: TestSuite[] = sampleTestSuites(NOW);

  it("test suites: test-first, steps below only when there are steps", () => {
    expect(trustSections({ suites, testRuns: 3, steps: 0 })).toEqual({ suites: true, testRuns: false, steps: false, empty: false });
    expect(trustSections({ suites, testRuns: 3, steps: 4 })).toEqual({ suites: true, testRuns: false, steps: true, empty: false });
  });

  it("test runs but no suites (an older server, or nothing recorded yet): the latest runs", () => {
    expect(trustSections({ suites: null, testRuns: 2, steps: 0 })).toEqual({ suites: false, testRuns: true, steps: false, empty: false });
    expect(trustSections({ suites: [], testRuns: 2, steps: 0 }).testRuns).toBe(true);
  });

  it("only step-level data: the steps, as before", () => {
    expect(trustSections({ suites: null, testRuns: 0, steps: 5 })).toEqual({ suites: false, testRuns: false, steps: true, empty: false });
  });

  it("nothing at all (or a server without the endpoints): the empty state, never an empty step table", () => {
    expect(trustSections({ suites: null, testRuns: 0, steps: 0 })).toEqual({ suites: false, testRuns: false, steps: false, empty: true });
    expect(trustSections({ suites: [{ ...suites[0], files: [], totals: { ...suites[0].totals, files: 0 } }], testRuns: 0, steps: 0 }).empty).toBe(true);
  });
});

describe("compact Trust: sorting, the 8-file rule, filters", () => {
  it("needs a look first (paused, then always runs), then most time saved, then by path", () => {
    const got = sortFiles([
      file({ file: "s-small", state: "skipping", savedMs: 100 }),
      file({ file: "r", state: "ready", savedMs: 0 }),
      file({ file: "a", state: "always", savedMs: 0 }),
      file({ file: "s-big", state: "skipping", savedMs: 9_000 }),
      file({ file: "p", state: "paused", savedMs: 50 }),
      file({ file: "l", state: "learning", savedMs: null }),
      file({ file: "s-mid", state: "skipping", savedMs: 2_000 }),
    ]).map((f) => f.file);
    expect(got).toEqual(["p", "a", "s-big", "s-mid", "s-small", "l", "r"]);
  });

  it("at most 8 files until Show all; small lists show everything", () => {
    const xs = Array.from({ length: 184 }, (_, i) => i);
    expect(MAX_FILES).toBe(8);
    expect(visibleFiles(xs, false)).toEqual({ shown: xs.slice(0, 8), hidden: 176 });
    expect(visibleFiles(xs, true)).toEqual({ shown: xs, hidden: 0 });
    expect(visibleFiles(xs.slice(0, 8), false).hidden).toBe(0);
    expect(visibleFiles([], false)).toEqual({ shown: [], hidden: 0 });
  });

  it("filter pill counts, with and without a search", () => {
    const files = [file({ file: "src/a.test.ts", state: "paused" }), file({ file: "src/b.test.ts", state: "always" }), file({ file: "lib/c.test.ts", state: "skipping" }), file({ file: "lib/d.test.ts", state: "ready" }), file({ file: "src/e.test.ts", state: "learning" })];
    expect(filterCounts(files)).toEqual({ all: 5, look: 2, skipping: 1, ready: 1, learning: 1 });
    expect(filterCounts(files, "  SRC/ ")).toEqual({ all: 3, look: 2, skipping: 0, ready: 0, learning: 1 });
    expect(filterCounts([])).toEqual({ all: 0, look: 0, skipping: 0, ready: 0, learning: 0 });
    expect(matchesFilter(files[0], "look")).toBe(true);
    expect(matchesFilter(files[4], "look")).toBe(false);
  });

  it("the sample's pill counts add up to its totals", () => {
    const all = sampleTestSuites(NOW).flatMap((s) => s.files);
    expect(filterCounts(all)).toEqual({ all: 296, look: 4, skipping: 265, ready: 17, learning: 10 });
  });
});

describe("compact Trust: % skipped is the median of the last 20 critical runs", () => {
  const run = (replayed: number, total = 100, kind: TestRunSummary["kind"] = "critical") => ({ kind, files: { total, replayed, ran: total - replayed, narrowed: 0 } });

  it("odd and even counts; shadow runs and empty runs do not count", () => {
    expect(medianSkippedPct([run(90), run(10), run(50)])).toBe(50);
    expect(medianSkippedPct([run(90), run(80)])).toBe(85);
    expect(medianSkippedPct([run(0, 100, "shadow"), run(40), run(0, 0)])).toBe(40);
  });

  it("only the newest 20 (runs come newest first)", () => {
    const newest = Array.from({ length: 20 }, () => run(95));
    const older = Array.from({ length: 30 }, () => run(0));
    expect(medianSkippedPct([...newest, ...older])).toBe(95);
  });

  it("no runs: null (shown as n/a)", () => {
    expect(medianSkippedPct([])).toBeNull();
    expect(medianSkippedPct([run(0, 100, "shadow")])).toBeNull();
  });
});

describe("compact Trust: the model", () => {
  const runs = sampleTestRuns(NOW).map(summaryOnly);
  const ago = () => "ago";

  it("matches runs to suites by repository and dir (the root '.' too)", () => {
    const [backend, , , root] = sampleTestSuites(NOW);
    expect(runsOfSuite(runs, backend).every((r) => r.suite === "packages/backend" && r.repository === "quodci/web")).toBe(true);
    expect(runsOfSuite(runs, { repository: "QUODCI/quod", suite: "./" })).toHaveLength(2);
    expect(runsOfSuite(runs, root)).toHaveLength(2);
  });

  it("summary, needs a look and suite rows from the sample", () => {
    const m = trustModel({ team: "quodci", suites: sampleTestSuites(NOW), runs, stats: null, now: NOW, ago });
    expect(m.summary.totals).toEqual({ files: 296, learning: 10, ready: 17, skipping: 265, always: 3, paused: 1 });
    expect(m.summary.bar.map((b) => b.state)).toEqual(["learning", "ready", "skipping", "always", "paused"]);
    expect(m.summary.wrongResults).toBe(0);
    expect(m.summary.skippedPct).toBeGreaterThan(80);
    expect(m.summary.savedWeekMs).toBeGreaterThan(0);
    expect(m.look.map((l) => [l.state, l.file])).toEqual([
      ["paused", "src/billing/discounts.test.ts"],
      ["always", "src/api/routes/login.test.ts"],
      ["always", "src/db/migrations.test.ts"],
      ["always", "src/components/date-picker.test.tsx"],
    ]);
    expect(m.look[0].href).toMatch(/^\/quodci\/web\/tests\/tr_/);
    const root = m.suites[3];
    expect(root.heading).toBe("quod · repo root");
    expect(root.recorded).toBe(false);
    expect(root.ledger).toBe(NO_RECORDING);
    expect(root.skippedPct).toBe(0);
    expect(m.suites[0].files.slice(0, 3).map((f) => f.state)).toEqual(["paused", "always", "always"]);
    expect(m.suites[0].lastRunAt).not.toBeNull();
  });

  it("none: no suites, no runs, no stats gives zeros and n/a, not errors", () => {
    const m = trustModel({ team: "t", suites: [], runs: [], stats: null, now: NOW, ago });
    expect(m.look).toEqual([]);
    expect(m.suites).toEqual([]);
    expect(m.summary).toMatchObject({ wrongResults: 0, spotChecksPassed: 0, savedWeekMs: 0, weekRuns: 0, skippedPct: null, bar: [] });
  });

  it("a suite without runs: no last run, n/a skipped, links to its test runs", () => {
    const s = { ...sampleTestSuites(NOW)[0], repository: "acme/other", repoSlug: "other" };
    const m = trustModel({ team: "acme", suites: [s], runs, stats: null, now: NOW, ago });
    expect(m.suites[0]).toMatchObject({ lastRunAt: null, skippedPct: null, savedWeekMs: 0 });
    expect(m.look[0].href).toBe("/acme/other/tests");
  });
});

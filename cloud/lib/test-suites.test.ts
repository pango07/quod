import { describe, expect, it } from "vitest";
import { sampleTestSuites } from "@/lib/api/fixture-tests";
import type { TestFile, TestSuite } from "@/lib/api/types";
import { fileStateLabel, lastResultText, splitFiles, stateCounts, suiteHeading, suiteLedgerText, suiteTotalsText, testFilesLine, trustSections } from "./test-suites";

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
    expect(suiteLedgerText(null, ago)).toMatch(/^No recording yet/);
  });

  it("totals: zeros left out, in a fixed order", () => {
    expect(stateCounts({ skipping: 152, ready: 9, learning: 0, always: 3, paused: 0 })).toBe("152 skipping, 9 ready, 3 always run");
    expect(suiteTotalsText({ files: 1, skipping: 0, ready: 0, learning: 1, always: 0, paused: 0, savedMs: 0 })).toBe("1 test file: 1 learning.");
    expect(suiteTotalsText({ files: 10, skipping: 10, ready: 0, learning: 0, always: 0, paused: 0, savedMs: 90_000 })).toBe("10 test files: 10 skipping. Saved 2 min.");
  });

  it("attention first (paused, always, learning); skipping and ready fold under them", () => {
    const { attention, rest } = splitFiles([file({ state: "paused", file: "p" }), file({ state: "always", file: "a" }), file({ state: "learning", file: "l" }), file({ state: "skipping", file: "s" }), file({ state: "ready", file: "r" })]);
    expect(attention.map((f) => f.file)).toEqual(["p", "a", "l"]);
    expect(rest.map((f) => f.file)).toEqual(["s", "r"]);
  });

  it("the Overview line adds every suite up; none: no line", () => {
    expect(testFilesLine(sampleTestSuites(NOW))).toBe("280 test files: 258 skipping, 17 ready, 1 learning, 3 always run, 1 paused");
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

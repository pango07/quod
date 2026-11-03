import { describe, expect, it } from "vitest";
import type { TestRunSummary } from "@/lib/api/types";
import { concernsTestRun, formatSavedMs, hasTestActivity, ledgerSentence, spotCheckSentence, suiteLabel, testRunHeadline } from "./test-runs";

const run = (over: Partial<TestRunSummary> = {}): TestRunSummary => ({
  id: "tr_1",
  at: "2026-11-02T12:00:00Z",
  repoSlug: "web",
  repository: "quodci/web",
  suite: "packages/backend",
  kind: "critical",
  tool: "vitest",
  workflow: "ci",
  job: "test",
  runId: 1,
  runAttempt: 1,
  runNumber: 1,
  githubUrl: null,
  branch: "main",
  sha: "9f3c2a17e4b80d6c",
  event: "push",
  files: { total: 184, ran: 13, replayed: 171, narrowed: 9 },
  timing: { wallMs: 21400, selectMs: 300, estPlainMs: 118900, savedMs: 79240 },
  exit: 0,
  ledger: null,
  audits: null,
  ...over,
});

describe("test run wording", () => {
  it("the headline", () => {
    expect(testRunHeadline(run())).toBe("vitest · packages/backend · 184 test files: 171 skipped (unchanged), 13 ran, saved 79 s");
    // The repo root shows the repository; no "saved" when it was not reported; one file is singular.
    expect(testRunHeadline(run({ suite: ".", files: { total: 1, ran: 1, replayed: 0, narrowed: 0 }, timing: { wallMs: null, selectMs: null, estPlainMs: null, savedMs: null } }))).toBe(
      "vitest · quodci/web · 1 test file: 0 skipped (unchanged), 1 ran",
    );
    expect(testRunHeadline(run({ files: { total: 1840, ran: 13, replayed: 1827, narrowed: 0 } }))).toContain("1,840 test files: 1,827 skipped (unchanged)");
  });

  it("saved time", () => {
    expect(formatSavedMs(0)).toBe("0 s");
    expect(formatSavedMs(420)).toBe("0.4 s");
    expect(formatSavedMs(79_240)).toBe("79 s");
    expect(formatSavedMs(12 * 60_000)).toBe("12 min");
    expect(formatSavedMs(3.4 * 3_600_000)).toBe("3.4 h");
    expect(formatSavedMs(null)).toBe("n/a");
    expect(suiteLabel({ suite: "", repository: "a/b" })).toBe("a/b");
  });

  it("spot-checks in one sentence", () => {
    const files = (oks: boolean[]) => oks.map((ok, i) => ({ file: `f${i}.test.ts`, ok }));
    expect(spotCheckSentence({ audits: null, auditFiles: [] })).toBeNull();
    expect(spotCheckSentence({ audits: { checked: 4, mismatched: 0 }, auditFiles: files([true, true, true, true]) })).toBe(
      "4 skipped files were re-run in the background to check: all matched.",
    );
    expect(spotCheckSentence({ audits: { checked: 1, mismatched: 0 }, auditFiles: [] })).toBe("1 skipped file was re-run in the background to check: it matched.");
    expect(spotCheckSentence({ audits: null, auditFiles: files([true, false, true]) })).toBe("3 skipped files were re-run in the background to check: 1 did not match.");
  });

  it("the reused recording", () => {
    expect(ledgerSentence(null)).toBe("Not reported for this run.");
    expect(ledgerSentence({ found: false, sha: null, branch: null, scope: null })).toMatch(/^No earlier recording was found/);
    expect(ledgerSentence({ found: true, sha: "1d7e5b2c90a4", branch: "main", scope: "main" })).toBe("Results reused from the recording of main at 1d7e5b2.");
  });

  it("a run page refreshes for its own report and its commit's background recording only", () => {
    const page = run();
    expect(concernsTestRun(run(), page)).toBe(true);
    expect(concernsTestRun(run({ id: "tr_2", kind: "shadow" }), page)).toBe(true);
    expect(concernsTestRun(run({ id: "tr_2", kind: "shadow", suite: "packages/web" }), page)).toBe(false);
    expect(concernsTestRun(run({ id: "tr_2", kind: "shadow", sha: "other" }), page)).toBe(false);
    expect(concernsTestRun(run({ id: "tr_2" }), page)).toBe(false);
  });

  it("the Overview card is hidden for a team without test runs", () => {
    expect(hasTestActivity(null)).toBe(false);
    const empty = { asOf: "", today: { savedMs: 0, runs: 0 }, week: { savedMs: 0, runs: 0, filesTotal: 0, filesSkipped: 0, skippedPct: null }, wrongResults: 0, audits: { checked: 0, passed: 0 }, recent: [] };
    expect(hasTestActivity(empty)).toBe(false);
    expect(hasTestActivity({ ...empty, recent: [run()] })).toBe(true);
  });
});

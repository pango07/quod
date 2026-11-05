import { describe, expect, it } from "vitest";
import type { TestRunSummary, TestStats } from "@/lib/api/types";
import {
  checkRuns,
  concernsTestRun,
  formatSavedMs,
  hasTestActivity,
  ledgerSentence,
  legacyRecordingIds,
  RECORDING_EXPLAINED,
  ROLE_EXPLAINED,
  roleLabel,
  roleTone,
  runRole,
  spotCheckSentence,
  statsWithRuns,
  suiteLabel,
  testRunHeadline,
} from "./test-runs";

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
    expect(testRunHeadline(run())).toBe("vitest · packages/backend · 184 test files: 171 skipped (unchanged), 13 ran, saved ≈ 79 s");
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
      "4 skipped files were run anyway, to check their reused results: all matched.",
    );
    expect(spotCheckSentence({ audits: { checked: 1, mismatched: 0 }, auditFiles: [] })).toBe("1 skipped file was run anyway, to check its reused result: it matched.");
    expect(spotCheckSentence({ audits: null, auditFiles: files([true, false, true]) })).toBe("3 skipped files were run anyway, to check their reused results: 1 did not match.");
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

describe("what a run was: inline recording (v2) and legacy background recordings", () => {
  const check = run({ id: "c1", kind: "critical", sha: "aaa" });
  const legacyShadow = run({ id: "s1", kind: "shadow", sha: "aaa", job: "record" });
  const inline = run({ id: "s2", kind: "shadow", sha: "bbb" });

  it("a shadow with a critical of the same repository, suite and commit is a legacy background recording; any other shadow is the developer's own job, recorded inline", () => {
    const legacy = legacyRecordingIds([check, legacyShadow, inline]);
    expect([...legacy]).toEqual(["s1"]);
    expect(runRole(check, legacy)).toBe("check");
    expect(runRole(legacyShadow, legacy)).toBe("legacy-recording");
    expect(runRole(inline, legacy)).toBe("recorded");
    // Another suite, another repository (any case), or no commit: not a pair.
    expect(legacyRecordingIds([check, { ...legacyShadow, suite: "packages/web" }]).size).toBe(0);
    expect(legacyRecordingIds([check, { ...legacyShadow, repository: "QUODCI/WEB" }]).size).toBe(1);
    expect(legacyRecordingIds([check, { ...legacyShadow, repository: "other/web" }]).size).toBe(0);
    expect(legacyRecordingIds([{ ...check, sha: null }, { ...legacyShadow, sha: null }]).size).toBe(0);
    // "." and "./" are the same suite.
    expect(legacyRecordingIds([{ ...check, suite: "." }, { ...legacyShadow, suite: "./" }]).size).toBe(1);
    expect(checkRuns([check, legacyShadow, inline]).map((r) => r.id)).toEqual(["c1", "s2"]);
  });

  it("labels and help in plain words", () => {
    expect(roleLabel("check")).toBe("Your tests");
    expect(roleLabel("recorded")).toBe("Your tests · recorded");
    expect(roleLabel("legacy-recording")).toBe("Legacy background recording");
    expect(roleTone("legacy-recording")).toBe("gray");
    expect(roleTone("recorded")).toBe("blue");
    expect(ROLE_EXPLAINED.recorded).toMatch(/in the same job/);
    expect(ROLE_EXPLAINED.recorded).toMatch(/second run of that file agrees/);
    expect(ROLE_EXPLAINED["legacy-recording"]).toMatch(/older setup/);
    expect(RECORDING_EXPLAINED).toMatch(/second, independent run/);
    for (const t of [ROLE_EXPLAINED.check, ROLE_EXPLAINED.recorded, RECORDING_EXPLAINED]) expect(t).not.toMatch(/background|shadow/i);
  });

  it("the Overview's stats: the server's count only critical runs; with inline runs among the runs, today, the week and recent come from them", () => {
    const now = Date.parse("2026-11-02T18:00:00Z");
    const server: TestStats = {
      asOf: "2026-11-02T18:00:00Z",
      today: { savedMs: 0, runs: 0 },
      week: { savedMs: 0, runs: 0, filesTotal: 0, filesSkipped: 0, skippedPct: null },
      wrongResults: 1,
      audits: { checked: 5, passed: 4 },
      recent: [],
    };
    const runs = [
      run({ id: "a", kind: "shadow", sha: "1", at: "2026-11-02T12:00:00Z", files: { total: 10, ran: 2, replayed: 8, narrowed: 0 }, timing: { wallMs: 1, selectMs: null, estPlainMs: null, savedMs: 4_000 } }),
      run({ id: "b", kind: "shadow", sha: "2", at: "2026-10-30T12:00:00Z", files: { total: 10, ran: 10, replayed: 0, narrowed: 0 }, timing: { wallMs: 1, selectMs: null, estPlainMs: null, savedMs: 0 } }),
      // Older than a week: in recent, not in the week.
      run({ id: "c", kind: "shadow", sha: "3", at: "2026-10-20T12:00:00Z", timing: { wallMs: 1, selectMs: null, estPlainMs: null, savedMs: 9_000 } }),
    ];
    const s = statsWithRuns(server, runs, now);
    expect(s.today).toEqual({ savedMs: 4_000, runs: 1 });
    expect(s.week).toEqual({ savedMs: 4_000, runs: 2, filesTotal: 20, filesSkipped: 8, skippedPct: 40 });
    expect(s.recent.map((r) => r.id)).toEqual(["a", "b", "c"]);
    // Spot-checks are the server's: it counts them from every shadow report already.
    expect([s.wrongResults, s.audits]).toEqual([1, { checked: 5, passed: 4 }]);
    expect(hasTestActivity(s)).toBe(true);
    // Only legacy runs (or none): the server's numbers as they are.
    expect(statsWithRuns(server, [check, legacyShadow], now)).toBe(server);
    expect(statsWithRuns(server, null, now)).toBe(server);
  });
});

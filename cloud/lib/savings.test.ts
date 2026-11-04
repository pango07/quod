import { describe, expect, it } from "vitest";
import { classify, formatMoney, formatSpan, mergeRuns, parseRate, plainTime, savingsModel, type TestRunLite } from "./savings";

const T0 = Date.parse("2026-11-01T00:00:00Z");
const H = 3_600_000;

function run(p: Partial<TestRunLite> & { id: string; hoursAfter: number }): TestRunLite {
  return {
    repoSlug: "web",
    repository: "acme/web",
    suite: "packages/api",
    kind: "critical",
    event: "push",
    branch: "main",
    runId: null,
    sha: null,
    files: { total: 100, ran: 100, replayed: 0 },
    timing: { wallMs: 60_000, estPlainMs: 50_000, savedMs: 0 },
    ...p,
    at: new Date(T0 + p.hoursAfter * H).toISOString(),
  };
}

describe("classify", () => {
  it("tells pull requests, feature branches and the default branch apart", () => {
    expect(classify({ event: "pull_request", branch: "feat/x" })).toBe("pr");
    expect(classify({ event: "push", branch: "refs/pull/12/merge" })).toBe("pr");
    expect(classify({ event: "push", branch: "main" })).toBe("default");
    expect(classify({ event: "push", branch: "dev" }, "dev")).toBe("default");
    expect(classify({ event: "push", branch: "main" }, "dev")).toBe("branch");
    expect(classify({ event: "push", branch: "feat/x" })).toBe("branch");
  });
});

describe("plainTime", () => {
  it("uses the suite's recorded full runs, scaled to the check's file count", () => {
    const full = run({ id: "1", hoursAfter: 0, kind: "shadow", timing: { wallMs: 100_000, estPlainMs: 90_000, savedMs: 0 } });
    const check = run({ id: "2", hoursAfter: 1, files: { total: 110, ran: 10, replayed: 100 }, timing: { wallMs: 12_000, estPlainMs: 99_000, savedMs: 90_000 } });
    const m = savingsModel([check, full], { from: T0, to: T0 + 2 * H, usdPerMinute: 0.006 });
    expect(m.methods["full-run"]).toBe(1);
    // 100 s for 100 files -> 110 s for 110; saved 110 - 12 = 98 s.
    expect(m.savedMs).toBe(98_000);
  });

  it("without a full run, adds the skipped files' recorded time to the check's own", () => {
    const check = run({ id: "2", hoursAfter: 1, files: { total: 10, ran: 2, replayed: 8 }, timing: { wallMs: 5_000, estPlainMs: 20_000, savedMs: 16_000 } });
    expect(plainTime(check, new Map())).toEqual({ actualMs: 5_000, plainMs: 21_000, method: "file-times" });
  });

  it("never saves below zero, and a check that ran everything saves nothing", () => {
    const full = run({ id: "1", hoursAfter: 0, kind: "shadow", timing: { wallMs: 10_000, estPlainMs: 9_000, savedMs: 0 } });
    const slow = run({ id: "2", hoursAfter: 1, files: { total: 100, ran: 50, replayed: 50 }, timing: { wallMs: 30_000, estPlainMs: 9_000, savedMs: 4_000 } });
    const all = run({ id: "3", hoursAfter: 1, suite: "packages/other" });
    const m = savingsModel([full, slow, all], { from: T0, to: T0 + 2 * H, usdPerMinute: 0.006 });
    expect(m.savedMs).toBe(0);
  });
});

describe("savingsModel", () => {
  it("subtracts the background recordings from the money, not from the time", () => {
    const shadow = run({ id: "1", hoursAfter: 1, kind: "shadow", timing: { wallMs: 60_000, estPlainMs: 50_000, savedMs: 0 } });
    const check = run({ id: "2", hoursAfter: 1, files: { total: 100, ran: 0, replayed: 100 }, timing: { wallMs: 0, estPlainMs: 50_000, savedMs: 50_000 } });
    const m = savingsModel([shadow, check], { from: T0, to: T0 + 2 * H, usdPerMinute: 0.01 });
    expect(m.savedMs).toBe(60_000);
    expect(m.recordingMs).toBe(60_000);
    expect(m.usd.saved).toBeCloseTo(0.01);
    expect(m.usd.net).toBeCloseTo(0);
    expect(m.files).toEqual({ skipped: 100, total: 100 });
    expect(m.unchangedShare).toBe(1);
  });

  it("only counts runs in the range, but uses older full runs for the plain time", () => {
    const old = run({ id: "1", hoursAfter: -48, kind: "shadow", timing: { wallMs: 100_000, estPlainMs: 90_000, savedMs: 0 } });
    const check = run({ id: "2", hoursAfter: 1, files: { total: 100, ran: 0, replayed: 100 }, timing: { wallMs: 10_000, estPlainMs: 90_000, savedMs: 90_000 } });
    const m = savingsModel([check, old], { from: T0, to: T0 + 2 * H, usdPerMinute: 0.006 });
    expect(m.recordingMs).toBe(0);
    expect(m.savedMs).toBe(90_000);
    expect(m.firstAt).toBe(old.at);
  });

  it("adds a CI run's suites up, splits runs by kind, and accumulates money per day", () => {
    const a = run({ id: "1", hoursAfter: 1, runId: 7, event: "pull_request", branch: "refs/pull/1/merge", files: { total: 10, ran: 0, replayed: 10 }, timing: { wallMs: 1_000, estPlainMs: 30_000, savedMs: 30_000 } });
    const b = run({ id: "2", hoursAfter: 1, runId: 7, suite: "packages/web", event: "pull_request", branch: "refs/pull/1/merge", files: { total: 10, ran: 0, replayed: 10 }, timing: { wallMs: 2_000, estPlainMs: 30_000, savedMs: 30_000 } });
    const c = run({ id: "3", hoursAfter: 30, runId: 8, files: { total: 10, ran: 0, replayed: 10 }, timing: { wallMs: 1_000, estPlainMs: 60_000, savedMs: 60_000 } });
    const m = savingsModel([a, b, c], { from: T0, to: T0 + 40 * H, usdPerMinute: 1 });
    expect(m.ciRuns).toBe(2);
    expect(m.byClass.pr).toEqual({ ciRuns: 1, typicalWithMs: 3_000, typicalWithoutMs: 63_000 });
    expect(m.byClass.default.ciRuns).toBe(1);
    expect(m.days.map((d) => d.day)).toEqual(["2026-11-01", "2026-11-02"]);
    expect(m.days[0].cumulativeUsd).toBeCloseTo(1);
    expect(m.days[1].cumulativeUsd).toBeCloseTo(2);
    expect(m.latest?.id).toBe("3");
  });

  it("is empty, not broken, without runs", () => {
    const m = savingsModel([], { from: T0, to: T0 + H, usdPerMinute: 0.006 });
    expect(m.checks).toBe(0);
    expect(m.unchangedShare).toBeNull();
    expect(m.latest).toBeNull();
  });
});

describe("words and inputs", () => {
  it("formats money and spans", () => {
    expect(formatMoney(12.476)).toBe("$12.48");
    expect(formatMoney(-0.42)).toBe("−$0.42");
    expect(formatMoney(0.001)).toBe("<$0.01");
    expect(formatSpan(41_000)).toBe("41 s");
    expect(formatSpan(12 * 60_000)).toBe("12 min");
    expect(formatSpan(192 * 60_000)).toBe("3 h 12 min");
  });

  it("accepts a rate above 0 and at most $1", () => {
    expect(parseRate("0.006")).toBe(0.006);
    expect(parseRate("$0.016")).toBe(0.016);
    expect(parseRate("0")).toBeNull();
    expect(parseRate("2")).toBeNull();
    expect(parseRate("abc")).toBeNull();
  });

  it("merges live runs over the server's, newest first, by id", () => {
    const a = run({ id: "1", hoursAfter: 1 });
    const b = run({ id: "2", hoursAfter: 2 });
    const a2 = { ...a, files: { total: 1, ran: 1, replayed: 0 } };
    expect(mergeRuns([a], [b, a2]).map((r) => [r.id, r.files.total])).toEqual([
      ["2", 100],
      ["1", 1],
    ]);
  });
});

describe("parseRate rounding", () => {
  it("rejects a rate that rounds to 0", () => {
    expect(parseRate("0.0000001")).toBeNull();
    expect(parseRate("0.006")).toBe(0.006);
  });
});

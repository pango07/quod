import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { RepoSummary, TestRunSummary } from "@/lib/api/types";
import { repoSkippedNote, testsByRepo } from "@/lib/test-runs";
import { RepoTable } from "./repo-table";

// The Overview's Repositories card on test-level data (/test-runs).

const NOW = Date.parse("2026-11-02T12:00:00Z");
const HOUR = 3_600_000;
const ago = (h: number) => new Date(NOW - h * HOUR).toISOString();
const text = (html: string) => html.replace(/<[^>]+>/g, " ").replace(/\s+/g, " ");

const run = (over: Partial<TestRunSummary> & { replayed?: number; total?: number }): TestRunSummary => {
  const { replayed = 5, total = 5, ...rest } = over;
  return {
    id: "tr_1",
    at: ago(1),
    repoSlug: "class-management",
    repository: "pango07/class-management",
    suite: "apps/landing",
    kind: "critical",
    tool: "vitest",
    workflow: "ci",
    job: "test",
    runId: 1,
    runAttempt: 1,
    runNumber: 274,
    githubUrl: null,
    branch: "feat/x",
    sha: "abc",
    event: "pull_request",
    files: { total, ran: total - replayed, replayed, narrowed: 0 },
    timing: { wallMs: 1000, selectMs: 10, estPlainMs: 10_000, savedMs: 60_000 },
    exit: 0,
    ledger: null,
    audits: null,
    ...rest,
  };
};

const repo = (over: Partial<RepoSummary>): RepoSummary =>
  ({ slug: "x", fullName: "pango07/x", language: "", runner: "github-hosted", replayRate: 0, savedMinutes: 0, lastRunAt: null, lastRunStatus: null, pausedSteps: 0, hasQuodData: false, ...over }) as RepoSummary;

// class-management PR #274: landing 5/5, web 72/73, media 6/7 skipped; an older week-old run and a shadow do not count.
const runs = [
  run({ id: "tr_l", suite: "apps/landing", replayed: 5, total: 5, at: ago(2) }),
  run({ id: "tr_w", suite: "apps/web", replayed: 72, total: 73, at: ago(2) }),
  run({ id: "tr_m", suite: "apps/media", replayed: 6, total: 7, at: ago(1), exit: 1 }),
  run({ id: "tr_s", kind: "shadow", replayed: 0, total: 85, at: ago(1.5) }),
  run({ id: "tr_old", replayed: 0, total: 85, at: ago(24 * 8) }),
  run({ id: "tr_b", repository: "pango07/ci-bench", repoSlug: "ci-bench", suite: ".", replayed: 40, total: 50, at: ago(30) }),
];

describe("test runs per repository", () => {
  const by = testsByRepo(runs, NOW);

  it("skipped: the repo's last 20 checks, files added up (shadows never count); saved: its checks of the last 7 days", () => {
    // Checks: 5/5, 72/73, 6/7 and an 8-day-old 0/85; the shadow's 0/85 is left out.
    expect(by["pango07/class-management"]).toMatchObject({ skipped: { replayed: 83, total: 170, checks: 4 }, savedMs: 180_000, runs: 3 });
    expect(by["pango07/class-management"].last?.id).toBe("tr_m");
    expect(by["pango07/ci-bench"]).toMatchObject({ skipped: { replayed: 40, total: 50, checks: 1 }, runs: 1 });
    expect(repoSkippedNote(by["pango07/class-management"].skipped)).toBe("83 of 170 files, last 4 checks");
    expect(repoSkippedNote({ replayed: 0, total: 0, checks: 0, pct: null })).toBe("No checks yet");
  });

  it("only old runs: a last run and the share, no 7-day time", () => {
    const old = testsByRepo([run({ at: ago(24 * 9) })], NOW)["pango07/class-management"];
    expect(old).toMatchObject({ skipped: { replayed: 5, total: 5, checks: 1 }, savedMs: 0, runs: 0 });
    expect(old.last).not.toBeNull();
  });
});

describe("the Repositories card", () => {
  const repos = [
    repo({ slug: "class-management", fullName: "Pango07/Class-Management" }),
    repo({ slug: "ci-bench", fullName: "pango07/ci-bench" }),
    repo({ slug: "quod", fullName: "pango07/quod", hasQuodData: true, replayRate: 0, savedMinutes: 0 }),
    repo({ slug: "empty", fullName: "pango07/empty" }),
  ];
  const html = renderToStaticMarkup(createElement(RepoTable, { team: "acme", repos, asOf: new Date(NOW).toISOString(), tests: testsByRepo(runs, NOW) }));
  const t = text(html);

  it("repos with test runs show test files skipped, time saved and the last run (matched by full name, any case)", () => {
    expect(t).toContain("49%");
    expect(t).toContain("Test files: 83 of 170 files, last 4 checks");
    expect(html).toContain('title="83 of 170 files across 4 checks"');
    expect(t).toContain("3 min");
    expect(t).toContain("Tests failed · 1h ago");
    expect(html).toContain('href="/acme/class-management/tests/tr_m"');
    expect(t).toContain("Test files: 40 of 50 files, last 1 check");
  });

  it("step-only repos keep the step columns; repos with neither say so; no nested links", () => {
    expect(t).toContain("Steps replayed");
    expect(t.match(/Appears once Quod runs in this repo/g)).toHaveLength(1);
    expect(html).not.toMatch(/<a [^>]*>(?:(?!<\/a>)[\s\S])*<a /);
  });

  it("without test runs the card is as before", () => {
    const plain = text(renderToStaticMarkup(createElement(RepoTable, { team: "acme", repos, asOf: new Date(NOW).toISOString() })));
    expect(plain).toContain("Steps replayed");
    expect(plain.match(/Appears once Quod runs in this repo/g)).toHaveLength(3);
  });
});

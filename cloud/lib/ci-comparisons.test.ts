import { describe, expect, it } from "vitest";
import type { GhJobView, GhRun } from "./github/pages";
import { CI_COMPARISON, compareCiJobs } from "./ci-comparisons";

const run: GhRun = {
  id: 100, repoId: CI_COMPARISON.repoId, repoFullName: CI_COMPARISON.repository, repositoryPrivate: false,
  workflow: CI_COMPARISON.workflow, title: "Compare cloud checks", runNumber: 1, runAttempt: 2,
  event: "workflow_dispatch", branch: "main", sha: "a".repeat(40), actor: "maintainer",
  status: "completed", conclusion: "success", outcome: "passed", htmlUrl: "https://untrusted.invalid",
  startedAt: "2026-11-08T10:00:00Z", finishedAt: "2026-11-08T10:05:00Z", durationSeconds: 300,
};
function jobs(): GhJobView[] {
  return [CI_COMPARISON.candidate, CI_COMPARISON.baseline].map((name, i) => ({
    id: 200 + i, name, status: "completed", outcome: "passed",
    startedAt: "2026-11-08T10:00:10Z", finishedAt: i ? "2026-11-08T10:04:10Z" : "2026-11-08T10:03:10Z",
    // Deliberately unrelated: the calculator must use complete raw lifetimes, not this summary or steps.
    durationSeconds: 1, runnerName: "GitHub Actions 1000006247", runnerGroup: "GitHub Actions", labels: ["ubuntu-latest", "Linux", "X64"],
    htmlUrl: "https://untrusted.invalid", steps: [],
    source: { runId: run.id, runAttempt: run.runAttempt, repoId: run.repoId, sha: run.sha, workflow: run.workflow },
  }));
}

describe("observed whole-job comparisons", () => {
  it("uses complete job timestamps from one source and attempt, and reconstructs GitHub links", () => {
    const result = compareCiJobs(run, jobs());
    expect(result).toMatchObject({
      runId: 100, attempt: 2, sha: run.sha, candidate: { seconds: 180 }, baseline: { seconds: 240 },
      savedSeconds: 60, cost: "public-standard-free", runUrl: "https://github.com/pango07/quod/actions/runs/100/attempts/2",
    });
    expect(result?.candidate.url).toBe("https://github.com/pango07/quod/actions/runs/100/job/200");
  });

  it("preserves regressions and zero differences", () => {
    const observations = jobs();
    observations[0].finishedAt = "2026-11-08T10:04:40Z";
    expect(compareCiJobs(run, observations)?.savedSeconds).toBe(-30);
    observations[0].finishedAt = observations[1].finishedAt;
    expect(compareCiJobs(run, observations)?.savedSeconds).toBe(0);
  });

  it.each([
    { repoId: 99 }, { repoFullName: "other/quod" }, { workflow: "CI" }, { event: "push" },
    { runAttempt: 0 }, { sha: "not-a-commit" }, { status: "in_progress" as const },
    { conclusion: "failure" }, { outcome: "failed" as const }, { startedAt: null },
  ])("does not infer a baseline for an ineligible run: %j", change => {
    expect(compareCiJobs({ ...run, ...change }, jobs())).toBeNull();
  });

  it.each([
    { runId: 99 }, { runAttempt: 1 }, { repoId: 99 }, { sha: "b".repeat(40) }, { workflow: "CI" },
  ])("rejects each mismatched raw job identity: %j", change => {
    const observations = jobs();
    observations[0].source = { ...observations[0].source!, ...change };
    expect(compareCiJobs(run, observations)).toBeNull();
  });

  it("requires exactly two distinct, completed successful jobs with valid intervals", () => {
    expect(compareCiJobs(run, jobs().slice(0, 1))).toBeNull();
    expect(compareCiJobs(run, [...jobs(), jobs()[0]])).toBeNull();
    for (const change of [
      { id: 201 }, { name: "cloud" }, { source: undefined }, { outcome: "skipped" as const },
      { outcome: "failed" as const }, { status: "in_progress" as const }, { finishedAt: null },
      { startedAt: "2026-11-08T09:59:59Z" }, { finishedAt: "2026-11-08T10:06:00Z" },
      { finishedAt: "2026-11-08T10:00:10Z" }, { finishedAt: "invalid" },
    ]) {
      const observations = jobs();
      Object.assign(observations[0], change);
      expect(compareCiJobs(run, observations)).toBeNull();
    }
  });

  it("does not turn missing/private visibility or nonstandard runner evidence into dollars", () => {
    for (const repositoryPrivate of [true, null, undefined]) {
      expect(compareCiJobs({ ...run, repositoryPrivate }, jobs())?.cost).toBe("unknown");
    }
    for (const change of [
      { runnerGroup: null }, { runnerGroup: "Larger runners" }, { runnerName: null }, { runnerName: "custom-runner" },
      { labels: [] }, { labels: ["ubuntu-24.04"] }, { labels: ["ubuntu-latest", "self-hosted"] },
      { labels: ["ubuntu-latest", "large"] },
    ]) {
      const observations = jobs();
      Object.assign(observations[1], change);
      expect(compareCiJobs(run, observations)).toMatchObject({ savedSeconds: 60, cost: "unknown" });
    }
  });
});

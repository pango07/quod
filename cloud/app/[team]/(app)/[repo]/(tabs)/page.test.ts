import { createElement, type ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiUnavailableError, type RepoDetail, type TestRunSummary } from "@/lib/api/types";
import { sampleTestRuns, summaryOnly } from "@/lib/api/fixture-tests";
import { FixtureCloudApi } from "@/lib/api/fixture";
import type { ObservedCiComparisons } from "@/lib/observed-ci-comparisons";
import RepoPage from "./page";

const mocks = vi.hoisted(() => ({ getRepo: vi.fn(), getActivity: vi.fn(), listTestRuns: vi.fn(), refresh: vi.fn() }));
vi.mock("@/lib/api", () => ({ api: () => mocks }));
vi.mock("@/lib/observed-ci-comparisons", () => ({ observedCiComparisons: async (): Promise<ObservedCiComparisons> => ({ visible: false, available: true, inspectedRuns: 0, pairs: [] }) }));
vi.mock("@/lib/dal", () => ({ requireTeam: async () => ({ team: { slug: "quodci" } }) }));
vi.mock("@/lib/ui", () => ({ plainUi: async () => true }));
vi.mock("@/components/live/parts", () => ({ LiveRefresh: (props: unknown) => (mocks.refresh(props), null), NowRunning: () => null }));
vi.mock("@/components/rerun-button", () => ({ RerunButton: () => null }));
vi.mock("@/components/steps-table", () => ({ StepsTable: () => createElement("div", null, "Step recordings") }));

const asOf = "2026-11-06T20:00:00Z";
const repo: RepoDetail = {
  asOf, defaultBranch: "main", latestRun: null, runTimes: [], steps: [],
  summary: { slug: "ci-bench", fullName: "pango07/ci-bench", language: "TypeScript", runner: "github-hosted", replayRate: 0, savedMinutes: 0, lastRunAt: null, lastRunStatus: null, pausedSteps: 0, hasQuodData: false },
};
const report: TestRunSummary = {
  ...summaryOnly(sampleTestRuns(Date.parse(asOf))[0]),
  id: "recording-1", kind: "shadow", repoSlug: "ci-bench", repository: "pango07/ci-bench", suite: "packages/backend",
  branch: "bench/cloud-reporting-codex", sha: "abcdef1234567", at: asOf, exit: 0,
  files: { total: 120, ran: 120, replayed: 0, narrowed: 0 },
  timing: { wallMs: 93000, selectMs: 10, savedMs: 0, estPlainMs: 200000 },
};

async function render() {
  const page = await RepoPage({ params: Promise.resolve({ team: "quodci", repo: "ci-bench" }), searchParams: Promise.resolve({}) });
  return renderToStaticMarkup(page as ReactElement);
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.getRepo.mockResolvedValue(repo);
  mocks.getActivity.mockResolvedValue({ asOf, jobs: [], simulated: false });
  mocks.listTestRuns.mockResolvedValue([report]);
});

describe("repository Overview test recordings", () => {
  it("shows a test-only repo even when the step API has no Quod data", async () => {
    const html = await render();
    expect(mocks.listTestRuns).toHaveBeenCalledWith("quodci", { repo: "ci-bench", limit: 100 });
    expect(html).toContain("Latest test run");
    expect(html).toContain("120");
    expect(html).toContain("Your tests · recorded");
    expect(html).toContain("No test files were reused in this run");
    expect(html).toContain("packages/backend");
    expect(html).toContain("bench/cloud-reporting-codex");
    expect(html).toContain('/quodci/ci-bench/tests/recording-1');
    expect(html).toContain('/quodci/ci-bench/tests"');
    expect(html).not.toContain("Appears once Quod runs");
    expect(html).not.toContain("No Quod reports received");
    expect(html).not.toContain("Step recordings");
    expect(mocks.refresh).toHaveBeenCalledWith({ scope: { kind: "repo", repo: "ci-bench" }, types: ["job.finished", "run.finished", "test.run"] });
  });

  it("uses test wall time and keeps execution estimates separate from CI savings", async () => {
    mocks.listTestRuns.mockResolvedValue([{ ...report, files: { total: 120, ran: 20, replayed: 100, narrowed: 0 }, timing: { wallMs: 20000, selectMs: 10, savedMs: 180000, estPlainMs: 200000 } }]);
    const html = await render();
    expect(html).toContain("20 s");
    expect(html).toContain("Estimated test execution avoided");
    expect(html).toContain("across all workers");
    expect(html).not.toContain("90%");
    expect(html).not.toContain("$");
  });

  it("does not turn missing timing or results into zero or a pass", async () => {
    mocks.listTestRuns.mockResolvedValue([{ ...report, exit: null, timing: { ...report.timing, wallMs: null } }]);
    const html = await render();
    expect(html).toContain("n/a");
    expect(html).toContain("Result not reported");
    expect(html).not.toContain("Tests passed");
  });

  it("reports a data outage without claiming the repository never used Quod", async () => {
    mocks.listTestRuns.mockRejectedValue(new ApiUnavailableError("test reports down"));
    const html = await render();
    expect(html).toContain("Quod data unavailable");
    expect(html).not.toContain("No Quod reports received");
    expect(html).not.toContain("Appears once Quod runs");
  });

  it("distinguishes an unsupported test-report endpoint from an empty history", async () => {
    mocks.listTestRuns.mockResolvedValue(null);
    const html = await render();
    expect(html).toContain("Test reports: not available yet");
    expect(html).not.toContain("No Quod reports received");
  });

  it("explains Cloud reporting when both report histories are empty", async () => {
    mocks.listTestRuns.mockResolvedValue([]);
    const html = await render();
    expect(html).toContain("No Quod reports received");
    expect(html).toContain("Cloud reporting is enabled");
    expect(html).not.toContain("Latest test run");
  });

  it("preserves existing step runs and filters when both kinds of report exist", async () => {
    const sample = await new FixtureCloudApi(undefined, 0).getRepo("quodci", "quod");
    expect(sample?.latestRun).toBeTruthy();
    mocks.getRepo.mockResolvedValue({ ...sample, summary: repo.summary });
    const html = await render();
    expect(html).toContain("Latest test run");
    expect(html).toContain("Latest step run");
    expect(html).toContain("Step recordings");
  });
});

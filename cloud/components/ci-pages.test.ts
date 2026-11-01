import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { GitHubLink, RunSummary } from "@/lib/api/types";
import type { GhJobView, GhRun, JobMinutes, SeenRunner } from "@/lib/github/pages";
import { ciState, QuodPending } from "./github-ci";
import { GitHubRunView } from "./github-run-view";
import { RunnersView } from "./runners-view";
import { RunsView, type GitHubRunsData } from "./runs-view";
import { GitHubUsage } from "./usage-view";

// Every page, for a team that is not linked, linked with no data yet, and
// linked with GitHub data (and the sample team where it differs). A linked
// team with data never sees "No runs yet" or "Connect repository".

const render = (el: Parameters<typeof renderToStaticMarkup>[0]) => renderToStaticMarkup(el);
const ASOF = "2026-11-01T12:00:00.000Z";
const LINKED: GitHubLink = { state: "linked", repos: 2, accounts: ["acme"] };
const UNLINKED: GitHubLink = { state: "unlinked", canInstall: true };

const RUN: GhRun = {
  id: 77,
  repoId: 9001,
  repoFullName: "acme/api",
  workflow: "CI",
  title: "Fix the flaky test",
  runNumber: 42,
  runAttempt: 1,
  event: "push",
  branch: "main",
  sha: "a".repeat(40),
  actor: "jamie",
  status: "completed",
  conclusion: "success",
  outcome: "passed",
  htmlUrl: "https://github.com/acme/api/actions/runs/77",
  startedAt: "2026-11-01T11:00:00.000Z",
  finishedAt: "2026-11-01T11:04:00.000Z",
  durationSeconds: 240,
};

const ghData = (runs: GhRun[], any = runs.length > 0): GitHubRunsData => ({
  runs,
  hasMore: false,
  query: { repoSlug: null, repoId: null, branch: null, status: null, page: 1 },
  any,
  repos: [{ slug: "api", fullName: "acme/api" }],
  slugOf: { "acme/api": "api" },
});

describe("ciState", () => {
  it("tells the three cases apart (and the sample team, and a missing database)", () => {
    expect(ciState(UNLINKED, false)).toBe("connect");
    expect(ciState(LINKED, false)).toBe("waiting");
    expect(ciState(LINKED, true)).toBe("data");
    expect(ciState(undefined, false)).toBe("sample");
    expect(ciState({ state: "not-configured" }, false)).toBe("no-db");
    expect(ciState({ state: "unavailable" }, false)).toBe("unavailable");
  });
});

describe("Runs", () => {
  const view = (state: ReturnType<typeof ciState>, github: GitHubRunsData | null, quodRuns: RunSummary[] = []) =>
    render(createElement(RunsView, { team: "acme", asOf: ASOF, state, quodRuns, github, base: "/acme/runs" }));

  it("not linked: Connect GitHub", () => {
    const html = view("connect", null);
    expect(html).toContain("Connect GitHub to see your runs");
    expect(html).toContain('href="/acme/connect"');
    expect(html).not.toContain("Connect repository");
  });

  it("linked, no runs yet: push to see them", () => {
    const html = view("waiting", ghData([]));
    expect(html).toContain("No runs yet");
    expect(html).toContain("Push to one of its repositories");
    expect(html).not.toContain("Connect");
  });

  it("linked with runs: the table, filters and a link to the run's page here", () => {
    const html = view("data", ghData([RUN]));
    expect(html).toContain("Fix the flaky test");
    expect(html).toContain("acme/api · CI #42 · main · aaaaaaa · jamie");
    expect(html).toContain('href="/acme/api/actions/77"');
    expect(html).toContain("Passed");
    expect(html).toContain("4m");
    expect(html).toMatch(/<form [^>]*action="\/acme\/runs" method="get"/);
    expect(html).not.toContain("No runs yet");
    expect(html).not.toContain("Connect repository");
  });

  it("filters that match nothing say so, with a way back", () => {
    const d = ghData([], true);
    d.query = { ...d.query, status: "failed" };
    const html = view("data", d);
    expect(html).toContain("No runs match these filters.");
    expect(html).not.toContain("No runs yet");
  });

  it("a run of a repo without a page here opens on GitHub", () => {
    const html = view("data", { ...ghData([{ ...RUN, repoFullName: "other/x" }]), slugOf: {} });
    expect(html).toContain('href="https://github.com/acme/api/actions/runs/77"');
    expect(html).toContain("(opens on GitHub)");
  });
});

describe("a GitHub run's page", () => {
  const jobs: GhJobView[] = [
    {
      id: 1,
      name: "test",
      status: "in_progress",
      outcome: "running",
      startedAt: "2026-11-01T11:59:00.000Z",
      finishedAt: null,
      durationSeconds: null,
      runnerName: "GitHub Actions 2",
      runnerGroup: "GitHub Actions",
      labels: ["ubuntu-latest"],
      htmlUrl: "",
      steps: [
        { number: 1, name: "Set up job", outcome: "passed", startedAt: "2026-11-01T11:59:00.000Z", finishedAt: "2026-11-01T11:59:05.000Z", seconds: 5 },
        { number: 2, name: "Run tests", outcome: "running", startedAt: "2026-11-01T11:59:05.000Z", finishedAt: null, seconds: null },
      ],
    },
  ];

  it("shows the jobs with their runner and steps, and links to GitHub", () => {
    const html = render(createElement(GitHubRunView, { team: "acme", repo: "api", run: { ...RUN, status: "in_progress", outcome: "running", durationSeconds: null }, jobs, asOf: ASOF }));
    expect(html).toContain("View on GitHub");
    expect(html).toContain('href="https://github.com/acme/api/actions/runs/77"');
    expect(html).toContain("GitHub Actions 2");
    expect(html).toContain("Set up job");
    expect(html).toContain("Run tests");
    expect(html).toContain("0 of 1");
    expect(html).toContain("Appears once Quod runs in this repo");
  });
});

describe("Runners", () => {
  const view = (state: ReturnType<typeof ciState>, seen: SeenRunner[] | null) => render(createElement(RunnersView, { team: "acme", asOf: ASOF, state, quod: [], seen }));
  const SEEN: SeenRunner[] = [{ key: "hosted:ubuntu-latest", name: "ubuntu-latest", kind: "github-hosted", labels: ["ubuntu-latest"], group: "GitHub Actions", lastSeen: "2026-11-01T11:50:00.000Z", jobs24h: 12, busy: 1 }];

  it("not linked: Connect GitHub, under Seen in your CI", () => {
    const html = view("connect", null);
    expect(html).toContain("Seen in your CI");
    expect(html).toContain("Connect GitHub to see your runners");
    expect(html).not.toContain("Connect repository");
  });

  it("linked, nothing yet: push", () => {
    expect(view("waiting", [])).toContain("No runners yet");
  });

  it("linked with jobs: the runners seen, busy now, jobs in 24 h", () => {
    const html = view("data", SEEN);
    expect(html).toContain("ubuntu-latest");
    expect(html).toContain("GitHub-hosted");
    expect(html).toContain("Busy (1)");
    expect(html).toContain("12");
    expect(html).not.toContain("No runners yet");
    expect(html).not.toContain("Connect repository");
  });
});

describe("Usage", () => {
  const minutes: JobMinutes = { days: [{ day: "2026-11-01", minutes: 42, jobs: 7 }], totalMinutes: 42, jobs: 7 };
  const view = (state: ReturnType<typeof ciState>, m: JobMinutes | null) => render(createElement(GitHubUsage, { team: "acme", plan: "Hobby", range: "7d", state, week: m, inRange: m }));

  it("linked with jobs: CI minutes this week from GitHub; Quod's numbers appear later", () => {
    const html = view("data", minutes);
    expect(html).toContain("CI minutes this week");
    expect(html).toContain(">42<");
    expect(html).toContain("7 jobs");
    expect(html).toContain("Appears once Quod runs in your CI");
  });

  it("not linked, or nothing yet", () => {
    expect(view("connect", null)).toContain("Connect GitHub to see your CI minutes");
    expect(view("waiting", { days: [], totalMinutes: 0, jobs: 0 })).toContain("No CI minutes yet");
  });
});

describe("Trust (Quod only)", () => {
  const view = (state: ReturnType<typeof ciState>) => render(createElement(QuodPending, { team: "acme", state }, "Trust follows the steps Quod replays."));
  it("says it appears once Quod runs, and points at what works today", () => {
    expect(view("connect")).toContain("Connect GitHub");
    const linked = view("data");
    expect(linked).toContain("Appears once Quod runs in your CI");
    expect(linked).toContain('href="/acme/runs"');
    expect(linked).toContain("Join early access");
    expect(linked).not.toContain("Nothing to trust yet");
  });
});

import type { GhJobView, GhRun } from "./github/pages";

/** The reviewed dogfood workflow's optional, same-source comparison cohort. */
export const CI_COMPARISON = {
  repoId: 1440517147,
  repository: "pango07/quod",
  workflow: "quod-cloud",
  candidate: "cloud (through quod)",
  baseline: "cloud (plain baseline)",
} as const;

export interface CiComparison {
  runId: number;
  attempt: number;
  sha: string;
  finishedAt: string;
  runUrl: string;
  candidate: { seconds: number; url: string };
  baseline: { seconds: number; url: string };
  /** Signed: regressions are negative, never clamped to zero. */
  savedSeconds: number;
  /** Runner compute only. No storage, subscription or modeled per-minute prices. */
  cost: "public-standard-free" | "unknown";
}

const positiveId = (n: number) => Number.isSafeInteger(n) && n > 0;
const stamp = (s: string | null) => s === null ? NaN : Date.parse(s);

export function isComparisonRun(run: GhRun): boolean {
  return run.repoId === CI_COMPARISON.repoId && run.repoFullName === CI_COMPARISON.repository &&
    run.workflow === CI_COMPARISON.workflow && run.event === "workflow_dispatch" &&
    positiveId(run.id) && positiveId(run.runAttempt) && /^[a-f0-9]{40}$/.test(run.sha) &&
    run.status === "completed" && run.conclusion === "success" && run.outcome === "passed";
}

// GitHub documents free standard hosted runners for public repositories;
// larger runners remain billable. Unknown/extra labels or groups stay unknown.
// https://docs.github.com/en/billing/concepts/product-billing/github-actions
function standardUbuntu(job: GhJobView): boolean {
  const labels = job.labels.map(label => label.toLowerCase());
  return job.runnerGroup === "GitHub Actions" && /^GitHub Actions \d+$/.test(job.runnerName ?? "") &&
    labels.includes("ubuntu-latest") && new Set(labels).size === labels.length &&
    labels.every(label => ["ubuntu-latest", "linux", "x64"].includes(label));
}

/** Complete runner lifetimes, including checkout/install/setup/post, from one latest attempt. */
export function compareCiJobs(run: GhRun, jobs: GhJobView[]): CiComparison | null {
  if (!isComparisonRun(run) || jobs.length !== 2 || new Set(jobs.map(job => job.id)).size !== 2) return null;
  const candidate = jobs.find(job => job.name === CI_COMPARISON.candidate);
  const baseline = jobs.find(job => job.name === CI_COMPARISON.baseline);
  if (!candidate || !baseline) return null;
  const runStart = stamp(run.startedAt), runEnd = stamp(run.finishedAt);
  if (!Number.isFinite(runStart) || !Number.isFinite(runEnd) || runEnd <= runStart) return null;
  for (const job of jobs) {
    const source = job.source;
    const start = stamp(job.startedAt), end = stamp(job.finishedAt);
    if (!positiveId(job.id) || job.status !== "completed" || job.outcome !== "passed" ||
        !source || source.runId !== run.id || source.runAttempt !== run.runAttempt ||
        source.repoId !== run.repoId || source.sha !== run.sha || source.workflow !== run.workflow ||
        !Number.isFinite(start) || !Number.isFinite(end) || start < runStart || end <= start || end > runEnd) return null;
  }
  // Generate source links from the verified identities, rather than trusting a payload URL.
  const runUrl = `https://github.com/${CI_COMPARISON.repository}/actions/runs/${run.id}`;
  const observed = (job: GhJobView) => ({ seconds: (stamp(job.finishedAt) - stamp(job.startedAt)) / 1000, url: `${runUrl}/job/${job.id}` });
  const throughQuod = observed(candidate), plain = observed(baseline);
  return {
    runId: run.id, attempt: run.runAttempt, sha: run.sha, finishedAt: run.finishedAt!,
    runUrl: `${runUrl}/attempts/${run.runAttempt}`, candidate: throughQuod, baseline: plain,
    savedSeconds: plain.seconds - throughQuod.seconds,
    cost: run.repositoryPrivate === false && jobs.every(standardUbuntu) ? "public-standard-free" : "unknown",
  };
}

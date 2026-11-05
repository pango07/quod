import type { JobSavings } from "./api/types";
import type { TestRunLite } from "./savings";
import { checkRuns } from "./test-runs";

/** Skipped file execution, never whole-job time or dollar savings. */
export function skippedExecution(runs: TestRunLite[] | null, from: number, to: number) {
  if (runs === null) return null;
  const unique = [...new Map(runs.map(r => [r.id, r])).values()];
  const checks = checkRuns(unique).filter(r => Date.parse(r.at) >= from && Date.parse(r.at) <= to);
  if (!checks.length) return null;
  const measured = checks.filter(r => r.timing.savedMs !== null && Number.isFinite(r.timing.savedMs) && r.timing.savedMs >= 0);
  return {minutes: measured.length ? measured.reduce((n, r) => n + r.timing.savedMs!, 0) / 60_000 : null, measured: measured.length, runs:checks.length};
}

/** Time and billing are two views of the same comparison, never additive percentages. */
export function savingsSummary(jobs: JobSavings | undefined) {
  if (!jobs || !Number.isInteger(jobs.jobs) || jobs.jobs <= 0 ||
      ![jobs.runnerSeconds, jobs.baselineRunnerSeconds, jobs.billedMinutes, jobs.baselineBilledMinutes].every(n => Number.isFinite(n) && n >= 0)) return null;
  const seconds = jobs.baselineRunnerSeconds - jobs.runnerSeconds;
  const minutes = jobs.baselineBilledMinutes - jobs.billedMinutes;
  const knownCost = [jobs.costUsd, jobs.baselineCostUsd].every(n => typeof n === "number" && Number.isFinite(n) && n >= 0);
  return {
    seconds,
    timeShare: jobs.baselineRunnerSeconds > 0 ? seconds / jobs.baselineRunnerSeconds : null,
    minutes,
    billingShare: jobs.baselineBilledMinutes > 0 ? minutes / jobs.baselineBilledMinutes : null,
    costUsd: knownCost ? jobs.baselineCostUsd! - jobs.costUsd! : null,
    jobs: jobs.jobs,
    estimated: jobs.baseline === "estimated",
  };
}

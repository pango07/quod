import type { JobSavings } from "./api/types";

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

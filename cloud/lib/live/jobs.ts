import type { LiveEvent, LiveJob, RunJob, StatsDelta, DayMinutes } from "@/lib/api/types";

// Pure reducers the browser applies to live events. Shared with tests.

/** "Now running": jobs queued or in progress, updated by job and step events. */
export function applyToJobs(jobs: LiveJob[], e: LiveEvent): LiveJob[] {
  switch (e.type) {
    case "job.queued":
    case "job.started":
      return [...jobs.filter((j) => j.id !== e.job.id), e.job].sort((a, b) => a.queuedAt.localeCompare(b.queuedAt));
    case "job.finished":
      return jobs.filter((j) => j.id !== e.job.id);
    case "step.started":
      return jobs.map((j) => (j.id === e.jobId ? { ...j, currentStep: { index: e.index, name: e.name } } : j));
    default:
      return jobs;
  }
}

/** The run page's steps, updated by step events for that run (they carry the RunStep), no refetch needed. */
export function applyToRunJobs(jobs: RunJob[], e: LiveEvent, repo: string, run: number): RunJob[] {
  if ((e.type !== "step.started" && e.type !== "step.finished") || e.repoSlug !== repo || e.run !== run) return jobs;
  return jobs.map((j) => {
    if (j.id !== e.jobId || !j.steps[e.index]) return j;
    const steps = [...j.steps];
    steps[e.index] =
      e.type === "step.finished"
        ? e.step
        : { ...steps[e.index], outcome: "running", seconds: 0, startedAt: e.at, why: { kind: "running", stepKey: e.stepKey } };
    return { ...j, steps };
  });
}

/** Adds a stats delta to the per-day series; a delta for a new (UTC) day opens a new bucket and drops the oldest. */
export function applyToDays(days: DayMinutes[], day: string, d: Pick<StatsDelta, "replayedMinutes" | "ranMinutes">): DayMinutes[] {
  if (!days.length) return days;
  const last = days[days.length - 1];
  if (last.day === day) return [...days.slice(0, -1), { ...last, replayedMinutes: last.replayedMinutes + d.replayedMinutes, ranMinutes: last.ranMinutes + d.ranMinutes }];
  if (day > last.day) return [...days.slice(1), { day, replayedMinutes: d.replayedMinutes, ranMinutes: d.ranMinutes }];
  return days; // an older day: not on this chart's last bar
}

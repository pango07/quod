// Live CI job events, as the GitHub App produces them. The SSE layer
// (CloudApi.subscribe, branch cloud/live) streams these next to Quod's own
// run, step and trust events. The shapes mirror cloud/live's `LiveJob` and
// `job.*` members of `LiveEvent` in lib/api/types.ts field for field; the
// extra fields (source, runId, runAttempt, labels, githubConclusion,
// repoFullName) are additions it can ignore. When the branches merge, keep
// one definition and re-export it from the other file.
//
// docs/api.md, "Event sources: GitHub App", is the prose version.

export type LiveJobStatus = "queued" | "in_progress" | "completed";

export interface LiveJob {
  /** GitHub's job id, as a string. */
  id: string;
  /** The repo's name on GitHub (no owner). Quod's repo slug where they match. */
  repoSlug: string;
  /** owner/name on GitHub. */
  repoName: string;
  /** The workflow run's number; 0 until the workflow_run delivery for it has arrived. */
  runNumber: number;
  /** Workflow name. */
  workflow: string;
  /** Job name. */
  job: string;
  branch: string;
  /** Head commit sha (40 hex). */
  sha: string;
  /** Who started the run (workflow_run actor), else the delivery's sender. */
  actor: string;
  /** runs-on labels: "ubuntu-latest", or "[self-hosted, quod]" for several. */
  runner: string;
  /** The job's page on GitHub. */
  githubUrl: string;
  status: LiveJobStatus;
  /** success, failure (also timed_out, startup_failure) or cancelled; absent for skipped, neutral and the rest (see githubConclusion). */
  conclusion?: "success" | "failure" | "cancelled";
  /** When GitHub queued the job (ISO 8601 UTC). */
  queuedAt: string;
  startedAt?: string;
  finishedAt?: string;
  /** The step GitHub last reported in progress. Coarse: GitHub sends no per-step webhooks (see docs). */
  currentStep?: { index: number; name: string };
  stepsTotal?: number;

  // Additions over cloud/live's LiveJob.
  source: "github";
  runId: number;
  runAttempt: number;
  labels: string[];
  /** GitHub's conclusion as sent (success, failure, cancelled, skipped, neutral, timed_out, action_required, startup_failure, stale). */
  githubConclusion?: string;
  /** Name of the runner that picked the job up, once started. */
  runnerName?: string;
}

export type LiveJobEventType = "job.queued" | "job.started" | "job.finished";

export interface LiveJobEvent {
  /** "gh-<live_version>": increasing across all GitHub job changes; usable as the SSE id. */
  id: string;
  /** When Quod stored the change (ISO 8601 UTC). */
  at: string;
  type: LiveJobEventType;
  job: LiveJob;
}

/** A workflow run, for "recent runs". */
export interface LiveRun {
  id: number;
  repoSlug: string;
  repoName: string;
  runNumber: number | null;
  runAttempt: number;
  workflow: string;
  title: string;
  branch: string;
  sha: string;
  actor: string;
  event: string;
  status: LiveJobStatus;
  githubConclusion?: string;
  githubUrl: string;
  startedAt: string | null;
  updatedAt: string;
}

export function eventTypeFor(status: LiveJobStatus): LiveJobEventType {
  return status === "completed" ? "job.finished" : status === "in_progress" ? "job.started" : "job.queued";
}

/** Parses "gh-123" back to 123 (for Last-Event-ID); null for anything else. */
export function versionOfEventId(id: string | null | undefined): number | null {
  const m = typeof id === "string" ? /^gh-(\d{1,15})$/.exec(id) : null;
  return m ? Number(m[1]) : null;
}

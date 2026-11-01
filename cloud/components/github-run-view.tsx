import Link from "next/link";
import { GitHubJobs, OutcomeDot, outcomeLabel } from "@/components/github-ci";
import { Elapsed } from "@/components/live/parts";
import { formatDuration, shortSha } from "@/lib/format";
import type { GhJobView, GhRun } from "@/lib/github/pages";

/** A GitHub workflow run: its header, timing, and jobs with their steps. Data in props. */
export function GitHubRunView({ team, repo, run, jobs, asOf }: { team: string; repo: string; run: GhRun; jobs: GhJobView[]; asOf: string }) {
  const done = jobs.filter((j) => j.status === "completed").length;
  const event = run.event === "pull_request" ? "pull request" : run.event.replaceAll("_", " ");
  return (
    <>
      <div className="row-between" style={{ alignItems: "flex-start", gap: 16 }}>
        <div style={{ display: "flex", flexDirection: "column", gap: 8, minWidth: 0 }}>
          <Link href={`/${team}/${repo}/runs`} className="link-muted" style={{ fontSize: 13 }}>
            {run.repoFullName} · Runs
          </Link>
          <h1 style={{ fontSize: 26, fontWeight: 600, letterSpacing: "-0.02em", lineHeight: 1.25, overflowWrap: "anywhere" }}>{run.title || run.workflow}</h1>
          <div style={{ display: "flex", gap: 14, alignItems: "center", fontSize: 13, color: "var(--muted)", flexWrap: "wrap" }}>
            <span style={{ display: "flex", alignItems: "center", gap: 6 }}>
              <OutcomeDot outcome={run.outcome} />
              {outcomeLabel(run.outcome)}
            </span>
            <span>
              {run.workflow}
              {run.runNumber ? ` #${run.runNumber}` : ""}
              {run.runAttempt > 1 ? `, attempt ${run.runAttempt}` : ""}
            </span>
            <span className="mono">
              {run.branch} · {shortSha(run.sha)}
            </span>
            {run.actor ? (
              <span>
                {event || "run"} by {run.actor}
              </span>
            ) : null}
          </div>
        </div>
        {run.htmlUrl ? (
          <a className="btn" href={run.htmlUrl} rel="noreferrer" target="_blank">
            View on GitHub
          </a>
        ) : null}
      </div>

      <section aria-label="Time" className="stats sm">
        <div className="card stat">
          <span className="label">{run.status === "in_progress" ? "Running for" : "Took"}</span>
          <span className="value">
            {run.status === "in_progress" && run.startedAt ? <Elapsed since={run.startedAt} asOf={asOf} /> : run.durationSeconds === null ? "–" : formatDuration(run.durationSeconds)}
          </span>
        </div>
        <div className="card stat">
          <span className="label">Jobs</span>
          <span className="value">{run.status === "completed" || !jobs.length ? jobs.length : `${done} of ${jobs.length}`}</span>
        </div>
        <div className="card stat">
          <span className="label">Steps replayed by Quod</span>
          <span className="value" style={{ color: "var(--muted)", fontSize: 15 }}>
            Appears once Quod runs in this repo
          </span>
        </div>
      </section>

      <GitHubJobs jobs={jobs} asOf={asOf} />
    </>
  );
}

import Link from "next/link";
import type { ReactNode } from "react";
import { Elapsed } from "@/components/live/parts";
import { Dot, EmptyState, OutcomeChip } from "@/components/ui";
import type { GitHubLink, RunStep } from "@/lib/api/types";
import { formatAgo, formatDuration, shortSha } from "@/lib/format";
import type { GhJobView, GhOutcome, GhRun, RunFilter, SeenRunner } from "@/lib/github/pages";
import { RUN_FILTERS } from "@/lib/github/pages";

// The dashboard's GitHub CI views (Runs, a GitHub run, Runners) and the
// empty states every page shares. Server components, data in props.

/**
 * Where a page stands for a team:
 * - "sample": the demo session's sample team (simulated CI);
 * - "connect": GitHub not linked;
 * - "no-db" / "unavailable": live CI cannot be read on this deploy, or right now;
 * - "waiting": linked, nothing from GitHub yet;
 * - "data": something to show.
 */
export type CiState = "sample" | "connect" | "no-db" | "unavailable" | "waiting" | "data";

export function ciState(link: GitHubLink | undefined, hasData: boolean): CiState {
  if (hasData) return "data";
  if (!link) return "sample";
  if (link.state === "linked") return "waiting";
  if (link.state === "unlinked") return "connect";
  return link.state === "not-configured" ? "no-db" : "unavailable";
}

/** The empty state for a page's state (never for "data"). `what` is plural: "runs", "runners". */
export function CiEmpty({ team, state, what, sample }: { team: string; state: CiState; what: string; sample?: ReactNode }) {
  switch (state) {
    case "connect":
      return (
        <EmptyState
          title={`Connect GitHub to see your ${what}`}
          action={
            <Link className="btn solid lg" href={`/${team}/connect`}>
              Connect GitHub
            </Link>
          }
        >
          Install the Quod CI GitHub App on your repositories and your CI shows here live, with or without Quod in it.
        </EmptyState>
      );
    case "waiting":
      return <EmptyState title={`No ${what} yet`}>GitHub is connected. Push to one of its repositories and the {what} show up here as they happen.</EmptyState>;
    case "no-db":
      return <EmptyState title={`No ${what} to show`}>Live CI needs a database, and this deploy has none yet.</EmptyState>;
    case "unavailable":
      return <EmptyState title={`Couldn't load ${what}`}>The database did not answer. Try again in a minute.</EmptyState>;
    case "sample":
      return <>{sample ?? <EmptyState title={`No ${what} yet`} />}</>;
    default:
      return null;
  }
}

const TONE: Record<GhOutcome, "green" | "red" | "blue" | "gray" | "dim"> = { passed: "green", failed: "red", running: "blue", queued: "gray", cancelled: "dim", skipped: "dim" };
const LABEL: Record<GhOutcome, string> = { passed: "Passed", failed: "Failed", running: "Running", queued: "Queued", cancelled: "Cancelled", skipped: "Skipped" };

export function OutcomeDot({ outcome }: { outcome: GhOutcome }) {
  return outcome === "queued" ? <span className="dot hollow" aria-hidden="true" /> : outcome === "running" ? <span className="dot blue pulse" aria-hidden="true" /> : <Dot tone={TONE[outcome]} />;
}

export const outcomeLabel = (o: GhOutcome) => LABEL[o];

function Took({ startedAt, seconds, running, asOf }: { startedAt: string | null; seconds: number | null; running: boolean; asOf: string }) {
  if (running && startedAt) return <Elapsed since={startedAt} asOf={asOf} />;
  return <>{seconds === null ? "–" : formatDuration(seconds)}</>;
}

/** Where a GitHub run opens: its page here when its repo has one, else GitHub. */
export function runHref(team: string, run: GhRun, slugOf: Record<string, string>): { href: string; external: boolean } {
  const slug = slugOf[run.repoFullName.toLowerCase()];
  return slug ? { href: `/${team}/${slug}/actions/${run.id}`, external: false } : { href: run.htmlUrl, external: true };
}

export function GitHubRunsTable({ team, runs, slugOf, asOf, showRepo }: { team: string; runs: GhRun[]; slugOf: Record<string, string>; asOf: string; showRepo?: boolean }) {
  return (
    <div>
      <div className="table-head cols-gh-runs" aria-hidden="true">
        <span>Run</span>
        <span>Status</span>
        <span>Took</span>
        <span>Started</span>
      </div>
      {runs.map((r) => {
        const { href, external } = runHref(team, r, slugOf);
        const body = (
          <>
            <span className="cell-main">
              <span style={{ display: "flex", alignItems: "center", gap: 8, minWidth: 0 }}>
                <OutcomeDot outcome={r.outcome} />
                <span className="ellipsis">{r.title || r.workflow}</span>
              </span>
              <span className="mono cell-sub ellipsis">
                {showRepo ? `${r.repoFullName} · ` : ""}
                {r.workflow}
                {r.runNumber ? ` #${r.runNumber}` : ""} · {r.branch} · {shortSha(r.sha)}
                {r.actor ? ` · ${r.actor}` : ""}
              </span>
            </span>
            <span style={{ fontSize: 13 }}>
              <span className="label-sm">Status </span>
              {outcomeLabel(r.outcome)}
            </span>
            <span className="mono" style={{ fontSize: 13 }}>
              <span className="label-sm" style={{ fontFamily: "var(--sans)" }}>
                Took{" "}
              </span>
              <Took startedAt={r.startedAt} seconds={r.durationSeconds} running={r.status === "in_progress"} asOf={asOf} />
            </span>
            <span style={{ fontSize: 13, color: "var(--muted)" }}>
              <span className="label-sm">Started </span>
              {r.startedAt ? <time dateTime={r.startedAt}>{formatAgo(r.startedAt, asOf)}</time> : "–"}
            </span>
          </>
        );
        return external ? (
          <a key={r.id} className="trow cols-gh-runs" href={href} rel="noreferrer" target="_blank">
            {body}
            <span className="sr-only"> (opens on GitHub)</span>
          </a>
        ) : (
          <Link key={r.id} className="trow cols-gh-runs" href={href}>
            {body}
          </Link>
        );
      })}
    </div>
  );
}

/** The Runs page's filters: a plain GET form (works without JavaScript, and on a phone). */
export function RunFilters({ action, repos, current }: { action: string; repos?: { slug: string; fullName: string }[]; current: { repo?: string; branch?: string; status?: RunFilter | null } }) {
  return (
    <form action={action} method="get" className="run-filters" aria-label="Filter runs">
      {repos ? (
        <label>
          <span className="label-xs">Repository</span>
          <select name="repo" defaultValue={current.repo ?? ""}>
            <option value="">All</option>
            {repos.map((r) => (
              <option key={r.slug} value={r.slug}>
                {r.fullName}
              </option>
            ))}
          </select>
        </label>
      ) : null}
      <label>
        <span className="label-xs">Branch</span>
        <input name="branch" defaultValue={current.branch ?? ""} placeholder="Any" maxLength={255} autoComplete="off" spellCheck={false} />
      </label>
      <label>
        <span className="label-xs">Status</span>
        <select name="status" defaultValue={current.status ?? ""}>
          <option value="">Any</option>
          {RUN_FILTERS.map((s) => (
            <option key={s} value={s}>
              {s[0].toUpperCase() + s.slice(1)}
            </option>
          ))}
        </select>
      </label>
      <button className="btn" type="submit">
        Filter
      </button>
    </form>
  );
}

export function Pager({ base, query, page, hasMore }: { base: string; query: Record<string, string | undefined>; page: number; hasMore: boolean }) {
  if (page <= 1 && !hasMore) return null;
  const href = (p: number) => {
    const q = new URLSearchParams(Object.entries({ ...query, page: p > 1 ? String(p) : undefined }).filter((e): e is [string, string] => !!e[1]));
    const s = q.toString();
    return s ? `${base}?${s}` : base;
  };
  return (
    <nav aria-label="Pages" className="pager">
      {page > 1 ? (
        <Link className="btn" href={href(page - 1)}>
          Newer
        </Link>
      ) : (
        <span />
      )}
      <span className="faint" style={{ fontSize: 13 }}>
        Page {page}
      </span>
      {hasMore ? (
        <Link className="btn" href={href(page + 1)}>
          Older
        </Link>
      ) : (
        <span />
      )}
    </nav>
  );
}

/** A GitHub run's jobs with their steps (from the stored webhook and REST data). */
/** `quod`: Quod's step for each GitHub step it handled, keyed `${jobId}:${stepNumber}` (lib/api/github-merge.ts). */
export function GitHubJobs({ jobs, asOf, quod }: { jobs: GhJobView[]; asOf: string; quod?: Map<string, RunStep> }) {
  if (!jobs.length) return <p className="muted">No jobs reported for this run yet.</p>;
  return (
    <ol className="gh-jobs" aria-label="Jobs">
      {jobs.map((j) => (
        <li key={j.id} className="card" style={{ overflow: "hidden" }}>
          <div className="card-head ruled" style={{ padding: "14px 18px", gap: 12, flexWrap: "wrap" }}>
            <h2 className="h2" style={{ fontSize: 15, display: "flex", alignItems: "center", gap: 8, minWidth: 0 }}>
              <OutcomeDot outcome={j.outcome} />
              <span className="sr-only">{outcomeLabel(j.outcome)}: </span>
              <span className="ellipsis">{j.name}</span>
            </h2>
            <span className="mono faint" style={{ fontSize: 12, display: "flex", gap: 10, flexWrap: "wrap" }}>
              <span>
                <Took startedAt={j.startedAt} seconds={j.durationSeconds} running={j.status === "in_progress"} asOf={asOf} />
              </span>
              <span>{j.runnerName ?? (j.labels.length ? j.labels.join(", ") : j.status === "queued" ? "waiting for a runner" : "")}</span>
            </span>
          </div>
          {j.steps.length ? (
            <ol className="gh-steps" aria-label={`Steps of ${j.name}`}>
              {j.steps.map((s) => {
                const q = quod?.get(`${j.id}:${s.number}`);
                return (
                <li key={s.number} className="step-line" data-state={s.outcome}>
                  <OutcomeDot outcome={s.outcome} />
                  <span className="ellipsis">{s.name}</span>
                  {q && (q.outcome === "replayed" || q.outcome === "ran") && s.outcome !== "failed" ? (
                    <span title={q.outcome === "replayed" ? "Quod gave back a proven result" : "Quod ran it and recorded what it read"}>
                      <OutcomeChip outcome={q.outcome} />
                    </span>
                  ) : (
                    <span className="faint" style={{ fontSize: 12 }}>
                      {s.outcome === "passed" ? "" : outcomeLabel(s.outcome)}
                    </span>
                  )}
                  <span className="mono" style={{ textAlign: "right", color: "var(--muted)" }}>
                    <Took startedAt={s.startedAt} seconds={s.seconds} running={s.outcome === "running"} asOf={asOf} />
                  </span>
                </li>
                );
              })}
            </ol>
          ) : (
            <p className="muted" style={{ padding: "12px 18px", fontSize: 13 }}>
              {j.status === "queued" ? "Waiting for a runner." : "Steps appear as GitHub reports them."}
            </p>
          )}
        </li>
      ))}
    </ol>
  );
}

export function RunnersSeen({ runners, asOf }: { runners: SeenRunner[]; asOf: string }) {
  return (
    <div>
      <div className="table-head cols-runners" aria-hidden="true">
        <span>Runner</span>
        <span>Jobs, 24 h</span>
        <span>Now</span>
        <span>Last seen</span>
      </div>
      <ul style={{ listStyle: "none", margin: 0, padding: 0 }}>
        {runners.map((r) => (
          <li key={r.key} className="trow cols-runners">
            <span className="cell-main">
              <span style={{ display: "flex", alignItems: "center", gap: 8, minWidth: 0 }}>
                <Dot tone={r.busy ? "blue" : "green"} />
                <span className="mono ellipsis">{r.name}</span>
              </span>
              <span className="cell-sub ellipsis">
                {r.kind === "github-hosted" ? "GitHub-hosted" : `Self-hosted${r.group ? ` · ${r.group}` : ""}`}
                {r.kind === "self-hosted" && r.labels.length ? ` · ${r.labels.join(", ")}` : ""}
              </span>
            </span>
            <span className="mono" style={{ fontSize: 13 }}>
              <span className="label-sm" style={{ fontFamily: "var(--sans)" }}>
                Jobs, 24 h{" "}
              </span>
              {r.jobs24h}
            </span>
            <span style={{ fontSize: 13 }}>
              <span className="label-sm">Now </span>
              {r.busy ? `Busy (${r.busy})` : "Idle"}
            </span>
            <span style={{ fontSize: 13, color: "var(--muted)" }}>
              <span className="label-sm">Last seen </span>
              <time dateTime={r.lastSeen}>{formatAgo(r.lastSeen, asOf)}</time>
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}

/**
 * For Quod-only pages (Trust, Quod's numbers) of a real team before Quod runs:
 * honest about Quod being in early access, and points at what works today.
 */
export function QuodPending({ team, state, title = "Appears once Quod runs in your CI", children }: { team: string; state: CiState; title?: string; children?: ReactNode }) {
  const linked = state === "waiting" || state === "data";
  return (
    <EmptyState
      title={title}
      action={
        <div style={{ display: "flex", gap: 10, flexWrap: "wrap", justifyContent: "center" }}>
          {state === "connect" ? (
            <Link className="btn solid lg" href={`/${team}/connect`}>
              Connect GitHub
            </Link>
          ) : linked ? (
            <Link className="btn solid lg" href={`/${team}/runs`}>
              See your runs
            </Link>
          ) : null}
          <a className="btn lg" href="https://www.quodci.com/#join">
            Join early access
          </a>
        </div>
      }
    >
      {children} {state === "connect" ? "Connect GitHub to see your CI live in the meantime." : linked ? "Your CI already shows live on Runs." : ""}
    </EmptyState>
  );
}

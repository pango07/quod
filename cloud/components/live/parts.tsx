"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import { MinutesChart } from "@/components/charts";
import { Stat } from "@/components/ui";
import type { Activity, DayMinutes, LiveEventType, LiveJob, LiveScope, TeamTotals } from "@/lib/api/types";
import { formatCount, formatDuration, formatMinutes, formatPercent, formatUsd, shortSha } from "@/lib/format";
import { inScope } from "@/lib/live/scope";
import { useLive, useLiveListener } from "./provider";

/** The current time, ticking every `ms`. */
function useNow(ms = 1000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), ms);
    return () => clearInterval(t);
  }, [ms]);
  return now;
}

/** Seconds since `since`, counting up every second (for running steps and runs). */
export function Elapsed({ since }: { since: string }) {
  const now = useNow();
  return <span suppressHydrationWarning>{formatDuration(Math.max(0, (now - Date.parse(since)) / 1000))}</span>;
}

/** The header indicator: says honestly whether updates are flowing. */
export function LiveBadge() {
  const { status, simulated } = useLive();
  const label = { connecting: "Connecting…", live: "Live", reconnecting: "Reconnecting…", paused: "Paused" }[status];
  const title =
    status === "paused"
      ? "Updates pause while this tab is hidden and resume when you come back."
      : status === "reconnecting"
        ? "Lost the live connection; retrying. What you see may be a few seconds old."
        : simulated
          ? "Sample data, simulated live: these runs are made up, not your CI."
          : "Updates arrive as your CI runs.";
  return (
    <span className={`live-badge ${status}`} title={title} role="status" aria-live="polite">
      <span className="live-dot" aria-hidden="true" />
      <span>{label}</span>
      {simulated && status === "live" ? <span className="live-sample">· Sample data, simulated live</span> : null}
      <span className="sr-only">. {title}</span>
    </span>
  );
}

const TYPES_THAT_CHANGE_PAGES: LiveEventType[] = ["run.started", "run.finished", "step.started", "step.finished", "trust.changed", "job.started", "job.finished"];

/**
 * Re-renders the page's server components (router.refresh) when an event in
 * `scope` arrives, at most once a second. The server is the source of truth;
 * the stream just says when to look again.
 */
export function LiveRefresh({ scope, types = TYPES_THAT_CHANGE_PAGES }: { scope: LiveScope; types?: LiveEventType[] }) {
  const router = useRouter();
  const pending = useRef<ReturnType<typeof setTimeout> | null>(null);
  const last = useRef(0);
  useLiveListener((e) => {
    if (!types.includes(e.type) || !inScope(e, scope)) return;
    if (pending.current) return;
    const wait = Math.max(0, 1000 - (Date.now() - last.current));
    pending.current = setTimeout(() => {
      pending.current = null;
      last.current = Date.now();
      router.refresh();
    }, wait);
  });
  useEffect(() => () => void (pending.current && clearTimeout(pending.current)), []);
  return null;
}

/** "Now running": jobs in progress (elapsed ticking, current step) and queued jobs (waiting time). */
export function NowRunning({ team, initial, repo }: { team: string; initial: Activity; repo?: string }) {
  const live = useLive();
  const now = useNow();
  const all = live.jobs ?? initial.jobs;
  const jobs = repo ? all.filter((j) => j.repoSlug === repo) : all;
  const running = jobs.filter((j) => j.status === "in_progress");
  const queued = jobs.filter((j) => j.status === "queued");
  const simulated = live.jobs ? live.simulated : initial.simulated;

  return (
    <section className="card" aria-labelledby="now-h" style={{ overflow: "hidden" }}>
      <div className="card-head">
        <h2 id="now-h" className="h2" style={{ display: "flex", alignItems: "center", gap: 8 }}>
          Now running
          <span className="chip small">{running.length}</span>
          {queued.length ? <span className="chip small">{queued.length} queued</span> : null}
        </h2>
        {simulated ? <span className="chip amber">Sample data, simulated live</span> : null}
      </div>
      {jobs.length === 0 ? (
        <p className="trow muted" style={{ display: "block", fontSize: 13 }}>
          Nothing running right now. Jobs show up here the moment GitHub queues them.
        </p>
      ) : (
        <ul className="now-list" aria-live="off">
          {[...running, ...queued].map((j) => (
            <JobRow key={j.id} team={team} job={j} now={now} showRepo={!repo} />
          ))}
        </ul>
      )}
    </section>
  );
}

function JobRow({ team, job, now, showRepo }: { team: string; job: LiveJob; now: number; showRepo: boolean }) {
  const queued = job.status === "queued";
  const since = Date.parse(queued ? job.queuedAt : (job.startedAt ?? job.queuedAt));
  const elapsed = formatDuration(Math.max(0, (now - since) / 1000));
  return (
    <li>
      <Link className="now-row" href={`/${team}/${job.repoSlug}/runs/${job.runNumber}`}>
        <span className={`dot lg ${queued ? "hollow" : "blue pulse"}`} aria-hidden="true" />
        <span className="cell-main" style={{ minWidth: 0 }}>
          <span style={{ fontWeight: 500 }}>
            {showRepo ? `${job.repoName} · ` : ""}
            {job.workflow} / {job.job} <span className="faint mono">#{job.runNumber}</span>
          </span>
          <span className="cell-sub">
            {queued ? (
              <>Waiting for a runner ({job.runner})</>
            ) : job.currentStep ? (
              <>
                Step {job.currentStep.index + 1} of {job.stepsTotal ?? "?"} · <span className="mono">{job.currentStep.name}</span>
              </>
            ) : (
              <>Starting on {job.runner}</>
            )}
          </span>
          <span className="cell-sub mono">
            {job.branch} · {shortSha(job.sha)} · {job.actor}
          </span>
        </span>
        <span className="now-time mono" title={queued ? "Time waiting in the queue" : "Time running"}>
          <span className="sr-only">{queued ? "Queued for " : "Running for "}</span>
          {/* The clock moves between server render and hydration; the count is meant to differ. */}
          <span suppressHydrationWarning>{queued ? `queued ${elapsed}` : elapsed}</span>
        </span>
      </Link>
    </li>
  );
}

/** Overview totals and the per-day chart, with each finished step's minutes added as it lands. */
export function LiveTotals({ totals, days, from }: { totals: TeamTotals; days: DayMinutes[]; from: string }) {
  const [t, setT] = useState(totals);
  const [d, setD] = useState(days);
  useLiveListener((e) => {
    if (e.type !== "stats.updated") return;
    const x = e.delta;
    setT((c) => ({
      ...c,
      savedMinutes: c.savedMinutes + x.replayedMinutes,
      costSavedUsd: (c.savedMinutes + x.replayedMinutes) * c.usdPerMinute,
      stepsReplayed: c.stepsReplayed + x.stepsReplayed,
      stepsTotal: c.stepsTotal + x.stepsTotal,
      spotChecks: c.spotChecks + x.spotChecks,
      spotChecksIdentical: c.spotChecksIdentical + x.spotChecksIdentical,
    }));
    setD((c) => {
      if (!c.length || c[c.length - 1].day !== e.day) return c;
      const last = c[c.length - 1];
      return [...c.slice(0, -1), { ...last, replayedMinutes: last.replayedMinutes + x.replayedMinutes, ranMinutes: last.ranMinutes + x.ranMinutes }];
    });
  });

  return (
    <>
      <section aria-label="Totals" className="stats">
        <Stat label="CI time saved" value={formatMinutes(t.savedMinutes)} note={`${formatCount(t.savedMinutes)} CI minutes you did not wait for`} />
        <Stat label="Steps replayed" value={formatPercent(t.stepsTotal ? t.stepsReplayed / t.stepsTotal : 0)} note={`${formatCount(t.stepsReplayed)} of ${formatCount(t.stepsTotal)} steps`} />
        <Stat label="Est. runner cost saved" value={formatUsd(t.costSavedUsd)} note={`at $${t.usdPerMinute} per Linux minute`} />
        <Stat
          label="Wrong replays"
          value={formatCount(t.wrongReplays)}
          note={
            t.spotChecks === t.spotChecksIdentical
              ? `${formatCount(t.spotChecks)} spot checks, all identical`
              : `${formatCount(t.spotChecks)} spot checks, ${formatCount(t.spotChecks - t.spotChecksIdentical)} different`
          }
        />
      </section>
      <section className="card" aria-labelledby="minutes-h" style={{ padding: 20, display: "flex", flexDirection: "column", gap: 16 }}>
        <div className="row-between">
          <h2 id="minutes-h" className="h2">
            CI minutes per day
          </h2>
          <div className="legend" aria-hidden="true">
            <span>
              <span className="swatch" style={{ background: "var(--blue)" }} />
              Replayed (saved)
            </span>
            <span>
              <span className="swatch" style={{ background: "var(--ran)" }} />
              Ran
            </span>
          </div>
        </div>
        <MinutesChart days={d.map((x) => ({ ...x, replayedMinutes: Math.round(x.replayedMinutes), ranMinutes: Math.round(x.ranMinutes) }))} from={from} />
      </section>
    </>
  );
}

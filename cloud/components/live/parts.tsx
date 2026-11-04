"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import { MinutesChart } from "@/components/charts";
import { Stat } from "@/components/ui";
import type { Activity, DayMinutes, LiveEventType, LiveJob, LiveScope, TeamTotals } from "@/lib/api/types";
import { formatCount, formatDuration, formatMinutes, formatPercent, formatUsd, shortSha } from "@/lib/format";
import { applyToDays } from "@/lib/live/jobs";
import { inScope } from "@/lib/live/scope";
import { useLive, useLiveListener } from "./provider";

/**
 * Where the server's clock (`start`, a render time) sits on this tab's
 * monotonic clock: recorded the first time a page's render time is seen, so
 * every clock on that page, including ones mounted later, agrees.
 */
const anchors = new Map<string, number>();
function anchor(start: string): { at: number; perf: number } {
  const at = Date.parse(start);
  if (typeof window === "undefined") return { at, perf: 0 };
  if (!anchors.has(start)) {
    if (anchors.size > 50) anchors.clear();
    anchors.set(start, performance.now());
  }
  return { at, perf: anchors.get(start)! };
}

/**
 * The current time on the server's clock, ticking every `ms`. It starts at
 * `start` (the server's render time), so the server HTML and the first client
 * render match, and moves on by the browser's monotonic clock
 * (performance.now), so a skewed browser clock never shows a wrong elapsed time.
 */
function useNow(start: string, ms = 1000): number {
  const [origin] = useState(() => anchor(start));
  // First render shows exactly `start` (so hydration matches); right after mount it catches up.
  const [now, setNow] = useState(origin.at);
  useEffect(() => {
    const tick = () => setNow(origin.at + (performance.now() - origin.perf));
    const first = setTimeout(tick, 0);
    const t = setInterval(tick, ms);
    return () => {
      clearTimeout(first);
      clearInterval(t);
    };
  }, [ms, origin]);
  return now;
}

/** Seconds since `since`, counting up every second (for running steps and runs). `asOf` is the server's render time. */
export function Elapsed({ since, asOf }: { since: string; asOf: string }) {
  const now = useNow(asOf);
  return <>{formatDuration(Math.max(0, (now - Date.parse(since)) / 1000))}</>;
}

const BADGE: Record<string, { label: string; title: string }> = {
  connecting: { label: "Connecting…", title: "Connecting to live updates." },
  live: { label: "Live", title: "Updates arrive as your CI runs." },
  reconnecting: { label: "Reconnecting…", title: "Live updates lost; retrying." },
  paused: { label: "Paused", title: "Live updates pause while this tab is hidden." },
  "signed-out": { label: "Signed out", title: "Sign in again to see live updates." },
  "no-access": { label: "No access", title: "You no longer have access to this team." },
};

/**
 * The header indicator: says honestly whether updates are flowing. The
 * provider only reports "Reconnecting" after 3 s, so blips and the planned
 * ~4 minute reconnect are never announced.
 */
export function LiveBadge() {
  const { status, simulated } = useLive();
  const b = BADGE[status];
  const sample = simulated && status === "live";
  return (
    <span className={`live-badge ${status}`} title={sample ? "Sample data, simulated live: these runs are made up, not your CI." : b.title}>
      <span className="live-dot" aria-hidden="true" />
      {/* Announced: a short, sustained state only. */}
      <span role="status">{b.label}</span>
      {sample ? <span className="live-sample">· Sample data, simulated live</span> : null}
    </span>
  );
}

/** Events after which a page's server-rendered parts are worth refetching: things that finished. */
const FINISHED: LiveEventType[] = ["job.finished", "run.finished"];
const REFRESH_DEBOUNCE_MS = 4_000;

/**
 * Re-renders the page's server components (router.refresh) after something in
 * `scope` finished, debounced: one refresh 4 s after the last such event.
 * Step-by-step changes are applied in the browser from the events themselves,
 * so a busy run never turns into a refresh per step (each refresh re-reads the
 * session and memberships).
 */
export function LiveRefresh({ scope, types = FINISHED }: { scope: LiveScope; types?: LiveEventType[] }) {
  const router = useRouter();
  const pending = useRef<ReturnType<typeof setTimeout> | null>(null);
  useLiveListener((e) => {
    if (!types.includes(e.type) || !inScope(e, scope)) return;
    if (pending.current) clearTimeout(pending.current);
    pending.current = setTimeout(() => {
      pending.current = null;
      router.refresh();
    }, REFRESH_DEBOUNCE_MS);
  });
  useEffect(() => () => void (pending.current && clearTimeout(pending.current)), []);
  return null;
}

/** "Now running": jobs in progress (elapsed ticking, current step) and queued jobs (waiting time). */
export function NowRunning({ team, initial, repo, bare }: { team: string; initial: Activity; repo?: string; bare?: boolean }) {
  const live = useLive();
  const now = useNow(initial.asOf);
  const all = live.jobs ?? initial.jobs;
  const jobs = repo ? all.filter((j) => j.repoSlug === repo) : all;
  const running = jobs.filter((j) => j.status === "in_progress");
  const queued = jobs.filter((j) => j.status === "queued");
  const simulated = live.jobs ? live.simulated : initial.simulated;

  if (bare) {
    // Inside the Overview's fold, which already says "Now running" and the counts.
    return (
      <div>
        {simulated ? (
          <p style={{ padding: "0 20px 12px" }}>
            <span className="chip amber">Sample data, simulated live</span>
          </p>
        ) : null}
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
      </div>
    );
  }

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
  // A job from GitHub has no Quod run page (yet): open it on GitHub.
  const external = job.source === "github" || !job.runNumber;
  const body = (
    <>
        <span className={`dot lg ${queued ? "hollow" : "blue pulse"}`} aria-hidden="true" />
        <span className="cell-main" style={{ minWidth: 0 }}>
          <span style={{ fontWeight: 500 }}>
            {showRepo ? `${job.repoName} · ` : ""}
            {job.workflow} / {job.job} {job.runNumber ? <span className="faint mono">#{job.runNumber}</span> : null}
            {external ? <span className="sr-only"> (opens on GitHub)</span> : null}
          </span>
          <span className="cell-sub">
            {queued ? (
              <>Waiting for a runner{job.runner ? ` (${job.runner})` : ""}</>
            ) : job.currentStep ? (
              <>
                Step {job.currentStep.index + 1} of {job.stepsTotal ?? "?"} · <span className="mono">{job.currentStep.name}</span>
              </>
            ) : (
              <>{job.runner ? `Starting on ${job.runner}` : "Starting"}</>
            )}
          </span>
          <span className="cell-sub mono">
            {job.branch} · {shortSha(job.sha)} · {job.actor}
          </span>
        </span>
        <span className="now-time mono" title={queued ? "Time waiting in the queue" : "Time running"}>
          {/* The ticking time is for eyes; screen readers get the state, not a number that changes every second. */}
          <span aria-hidden="true">{queued ? `queued ${elapsed}` : elapsed}</span>
          <span className="sr-only">{queued ? "queued" : "running"}</span>
        </span>
    </>
  );
  return (
    <li>
      {external ? (
        <a className="now-row" href={job.githubUrl} target="_blank" rel="noreferrer">
          {body}
        </a>
      ) : (
        <Link className="now-row" href={`/${team}/${job.repoSlug}/runs/${job.runNumber}`}>
          {body}
        </Link>
      )}
    </li>
  );
}

/**
 * Overview totals and the per-day chart, with each finished step's minutes
 * added as it lands. Mount it with key={range}: each range starts from its own
 * server totals.
 *
 * `reseed`: when the backend is authoritative (its totals already include
 * every event up to the render), a refresh replaces the client's running sum
 * with the server's. Off for the simulator: its overview folds in today's
 * deltas by the clock at render time while the stream delivers the same
 * deltas by event a moment later, so re-seeding on each refresh would count
 * the in-flight ones twice.
 */
/**
 * `tiles: false` leaves out the step-level tiles (a team with test data sees test numbers instead).
 * `daysFrom: "tests"`: the per-day chart is fed from test check runs (testMinutesPerDay), so step-level
 * deltas never touch it and each refresh (a test.run) replaces it.
 */
export function LiveTotals({
  totals,
  days,
  from,
  reseed = false,
  plain,
  tiles = true,
  daysFrom = "steps",
}: {
  totals: TeamTotals;
  days: DayMinutes[];
  from: string;
  reseed?: boolean;
  plain?: boolean;
  tiles?: boolean;
  daysFrom?: "steps" | "tests";
}) {
  const tests = daysFrom === "tests";
  const [t, setT] = useState(totals);
  const [d, setD] = useState(days);
  const [seen, setSeen] = useState({ totals, days });
  if ((reseed || tests) && (seen.totals !== totals || seen.days !== days)) {
    // Adjusting state while rendering when props change (react.dev, "storing information from previous renders").
    setSeen({ totals, days });
    setT(totals);
    setD(days);
  }
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
    if (!tests) setD((c) => applyToDays(c, e.day, x));
  });

  return (
    <>
      {tiles ? (
      <section aria-label="Totals" className="stats">
        <Stat label="CI time saved" value={formatMinutes(t.savedMinutes)} note={`${formatCount(t.savedMinutes)} CI minutes you did not wait for`} />
        <Stat label={plain ? "Steps skipped" : "Steps replayed"} value={formatPercent(t.stepsTotal ? t.stepsReplayed / t.stepsTotal : 0)} note={`${formatCount(t.stepsReplayed)} of ${formatCount(t.stepsTotal)} steps`} />
        <Stat label="Est. runner cost saved" value={formatUsd(t.costSavedUsd)} note={`at $${t.usdPerMinute} per Linux minute`} />
        <Stat
          label={plain ? "Wrong skips" : "Wrong replays"}
          value={formatCount(t.wrongReplays)}
          note={
            plain
              ? t.spotChecks === t.spotChecksIdentical
                ? `${formatCount(t.spotChecks)} re-checks, all the same output`
                : `${formatCount(t.spotChecks)} re-checks, ${formatCount(t.spotChecks - t.spotChecksIdentical)} different`
              : t.spotChecks === t.spotChecksIdentical
              ? `${formatCount(t.spotChecks)} spot checks, all identical`
              : `${formatCount(t.spotChecks)} spot checks, ${formatCount(t.spotChecks - t.spotChecksIdentical)} different`
          }
        />
      </section>
      ) : null}
      <section className="card" aria-labelledby="minutes-h" style={{ padding: 20, display: "flex", flexDirection: "column", gap: 16 }}>
        <div className="row-between">
          <h2 id="minutes-h" className="h2">
            {tests ? "Test check time per day" : "CI minutes per day"}
          </h2>
          <div className="legend" aria-hidden="true">
            <span>
              <span className="swatch" style={{ background: "var(--blue)" }} />
              {tests ? "≈ saved" : plain ? "Skipped (saved)" : "Replayed (saved)"}
            </span>
            <span>
              <span className="swatch" style={{ background: "var(--ran)" }} />
              Ran
            </span>
          </div>
        </div>
        {tests ? (
          <MinutesChart days={d} from={from} source="tests" />
        ) : (
          <MinutesChart days={d.map((x) => ({ ...x, replayedMinutes: Math.round(x.replayedMinutes), ranMinutes: Math.round(x.ranMinutes) }))} from={from} />
        )}
      </section>
    </>
  );
}

"use client";

// The Overview's answer to "what is Quod doing for us": money and CI time
// saved, test files skipped and unchanged inputs, then CI time per run by
// kind of run and money saved over time. Computed in the browser from the
// same runs the server rendered with (lib/savings.ts), and again whenever a
// test run arrives on the live stream, so the numbers move as CI finishes.

import Link from "next/link";
import { useEffect, useMemo, useState } from "react";
import { useLive, useLiveListener } from "@/components/live/provider";
import type { TestRunLite } from "@/lib/api/types";
import { formatAgo, formatCount, formatDay, formatDuration } from "@/lib/format";
import {
  changeText,
  CLASS_HELP,
  CLASS_LABEL,
  formatMoney,
  formatSpan,
  lite,
  mergeRuns,
  rateText,
  RUN_CLASSES,
  type RunClass,
  SAVINGS_METHOD,
  savingsModel,
  type SavingsModel,
} from "@/lib/savings";
import { suiteLabel } from "@/lib/test-runs";

const CLASS_ONE: Record<RunClass, string> = { pr: "pull request", branch: "feature branch", default: "default branch" };

export interface SavingsProps {
  team: string;
  runs: TestRunLite[];
  /** The range's start and the server's clock at render (ms since epoch / ISO). */
  from: number;
  asOf: string;
  rangeLabel: string;
  usdPerMinute: number;
  rateSource: "team" | "default";
  /** Lower-case owner/name to its default branch, when GitHub told us. */
  defaultBranches: Record<string, string | null>;
  /** "history": every run in the range (GET /test-history); "recent": only the newest 200 (an older server). */
  source: "history" | "recent";
}

/** The runs the server rendered with, plus any that arrived live since. */
function useLiveRuns(runs: TestRunLite[]): { all: TestRunLite[]; latestLive: { run: TestRunLite; at: number } | null } {
  const [extra, setExtra] = useState<TestRunLite[]>([]);
  const [latestLive, setLatestLive] = useState<{ run: TestRunLite; at: number } | null>(null);
  useLiveListener((e) => {
    if (e.type !== "test.run") return;
    const r = lite(e.run);
    setExtra((x) => mergeRuns(x, [r]));
    if (r.kind === "critical") setLatestLive({ run: r, at: Date.now() });
  });
  const all = useMemo(() => mergeRuns(runs, extra), [runs, extra]);
  return { all, latestLive };
}

/** Re-renders every `ms` so "just now" moves on. */
function useTick(ms: number): number {
  const [t, setT] = useState(0);
  useEffect(() => {
    const id = setInterval(() => setT((x) => x + 1), ms);
    return () => clearInterval(id);
  }, [ms]);
  return t;
}

export function Savings(p: SavingsProps) {
  const { all, latestLive } = useLiveRuns(p.runs);
  const { status } = useLive();
  useTick(15_000);
  // The window ends now: the server's render time, or later once live runs land.
  const newest = all[0]?.at ?? p.asOf;
  const to = Math.max(Date.parse(p.asOf), Date.parse(newest));
  const model = useMemo(() => savingsModel(all, { from: p.from, to, usdPerMinute: p.usdPerMinute, defaultBranches: p.defaultBranches }), [all, p.from, to, p.usdPerMinute, p.defaultBranches]);
  const nowIso = new Date(Math.max(to, latestLive?.at ?? 0)).toISOString();
  return (
    <>
      <SavingsHero p={p} m={model} live={status === "live"} latestLive={latestLive} nowIso={nowIso} />
      <div className="savings-charts">
        <CiTimeCard m={model} from={p.from} />
        <MoneyCard m={model} />
      </div>
    </>
  );
}

function Info({ text, label = "How this is worked out" }: { text: string; label?: string }) {
  return (
    <details className="info">
      <summary aria-label={label} title={label}>
        <svg aria-hidden="true" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
          <circle cx="12" cy="12" r="9" />
          <path d="M12 11v6M12 7.5v.5" strokeLinecap="round" />
        </svg>
      </summary>
      <div className="info-pop">{text}</div>
    </details>
  );
}

function SavingsHero({ p, m, live, latestLive, nowIso }: { p: SavingsProps; m: SavingsModel; live: boolean; latestLive: { run: TestRunLite; at: number } | null; nowIso: string }) {
  const latest = m.latest;
  const flash = latestLive && latest && latestLive.run.id === latest.id;
  const negative = m.usd.net < 0;
  const rate = `${rateText(m.usdPerMinute)}${p.rateSource === "default" ? ", GitHub's Linux 2-core price" : ", your team's price"}`;
  const nothing = m.checks === 0;
  return (
    <section className="card savings" aria-labelledby="savings-h">
      <div className="savings-head">
        <h2 id="savings-h" className="h2">
          What Quod saved you
        </h2>
        <span className="savings-when">
          {p.rangeLabel}
          {live ? (
            <span className="savings-live" title="Updates as your CI finishes test runs">
              <span className="dot blue pulse sm" aria-hidden="true" />
              Live
            </span>
          ) : null}
        </span>
      </div>

      {nothing ? (
        <p className="muted" style={{ padding: "4px 24px 24px", fontSize: 14, lineHeight: 1.6, maxWidth: "70ch" }}>
          No test checks in this period yet. When your CI runs tests through Quod, the money and time it saves show up here as each run finishes.
        </p>
      ) : (
        <div className="savings-body">
          <div className="savings-money">
            <span className="savings-label">
              Money saved on CI runners <span className="faint">(estimate)</span>
              <Info text={SAVINGS_METHOD} />
            </span>
            <span className={`savings-big tnum ${negative ? "neg" : ""}`} aria-live="polite">
              {formatMoney(m.usd.net)}
            </span>
            {m.recordingMs > 0 ? (
              <span className="savings-sum tnum" aria-label={`${formatMoney(m.usd.saved)} saved on checks, minus ${formatMoney(m.usd.recording)} for background recordings`}>
                <span>
                  {formatMoney(m.usd.saved)} <span className="faint">saved on checks</span>
                </span>
                <span>
                  − {formatMoney(m.usd.recording)} <span className="faint">background recordings</span>
                </span>
              </span>
            ) : null}
            <span className="savings-note">
              Runner minutes saved, priced at {rate}.{" "}
              <Link href={`/${p.team}/settings#savings`} className="link-muted" style={{ textDecoration: "underline", textUnderlineOffset: 2 }}>
                Change the price
              </Link>
            </span>
            {negative ? (
              <span className="savings-warn">Background recordings used more runner time than the checks saved in this period.</span>
            ) : null}
          </div>

          <dl className="savings-kpis">
            <div>
              <dt>CI time saved</dt>
              <dd className="tnum">{formatSpan(m.savedMs)}</dd>
              <dd className="savings-why">Less time your checks spent running tests, over {formatCount(m.ciRuns)} CI run{m.ciRuns === 1 ? "" : "s"}.</dd>
            </div>
            <div>
              <dt>Test files skipped</dt>
              <dd className="tnum">{formatCount(m.files.skipped)}</dd>
              <dd className="savings-why">Not run again, because nothing they depend on had changed. Their earlier passing result was reused.</dd>
            </div>
            <div>
              <dt>Unchanged inputs</dt>
              <dd className="tnum">{m.unchangedShare === null ? "n/a" : `${Math.round(m.unchangedShare * 100)}%`}</dd>
              <dd className="savings-why">
                Of the {formatCount(m.files.total)} test files your checks had, this share depended on code that had not changed since they last passed.
              </dd>
            </div>
          </dl>
        </div>
      )}

      {latest ? (
        <p className={`savings-latest ${flash ? "flash" : ""}`} key={latest.id}>
          <span>
            <span className={`dot sm ${flash ? "blue" : "gray"}`} aria-hidden="true" style={{ marginRight: 8, verticalAlign: "middle" }} />
            Latest check: <strong>{latest.repository.split("/")[1] ?? latest.repository}</strong> · {suiteLabel(latest)} · {CLASS_ONE[latest.cls]}
            {" · "}
            {latest.filesSkipped} of {latest.filesTotal} files skipped
            {latest.savedMs > 0 ? <> · saved {formatSpan(latest.savedMs)}</> : null}
          </span>
          <time className="faint" dateTime={latest.at}>
            {formatAgo(latest.at, nowIso)}
          </time>
        </p>
      ) : null}
      {p.source === "recent" ? (
        <p className="savings-foot faint">Counted from the newest 200 test runs; older ones in this period are not included yet.</p>
      ) : null}
    </section>
  );
}

// ---------------------------------------------------------------- Charts

type ClassFilter = RunClass | "all";

const W = 600;
const H = 180;

function path(points: { x: number; y: number }[]): string {
  return points.map((p, i) => `${i ? "L" : "M"}${p.x.toFixed(1)},${p.y.toFixed(1)}`).join("");
}

function CiTimeCard({ m, from }: { m: SavingsModel; from: number }) {
  const [cls, setCls] = useState<ClassFilter>("all");
  const days = m.days;
  const n = days.length;
  const xs = (i: number) => (n <= 1 ? W / 2 : (i / (n - 1)) * W);
  const vals = days.flatMap((d) => [d.with[cls], d.without[cls]]).filter((v): v is number => v !== null);
  const max = Math.max(1, ...vals) * 1.1;
  const ys = (v: number) => H - (v / max) * H;
  const withPts = days.map((d, i) => (d.with[cls] === null ? null : { x: xs(i), y: ys(d.with[cls]!) })).filter((p): p is { x: number; y: number } => !!p);
  const withoutPts = days.map((d, i) => (d.without[cls] === null ? null : { x: xs(i), y: ys(d.without[cls]!) })).filter((p): p is { x: number; y: number } => !!p);
  const firstAt = m.firstAt ? Date.parse(m.firstAt) : null;
  const adoptIdx = firstAt !== null && firstAt >= from ? days.findIndex((d) => d.day === new Date(firstAt).toISOString().slice(0, 10)) : -1;
  const counts = { all: m.ciRuns, ...Object.fromEntries(RUN_CLASSES.map((k) => [k, m.byClass[k].ciRuns])) } as Record<ClassFilter, number>;
  const tabs: [ClassFilter, string][] = [["all", "All runs"], ...RUN_CLASSES.map((k) => [k, CLASS_LABEL[k]] as [ClassFilter, string])];
  const has = withPts.length > 0;
  return (
    <section className="card chart-card" aria-labelledby="citime-h">
      <div className="chart-card-head">
        <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          <h2 id="citime-h" className="h2">
            Test time per CI run
          </h2>
          <p className="faint" style={{ fontSize: 12, lineHeight: 1.5 }}>
            The typical CI run each day: its test time with Quod, and without it (estimated).
          </p>
        </div>
        <div className="legend" aria-hidden="true">
          <span>
            <span className="swatch" style={{ background: "var(--blue)" }} />
            With Quod
          </span>
          <span>
            <span className="swatch dashed" />
            Without (est.)
          </span>
        </div>
      </div>
      <div className="pill-filters" role="group" aria-label="Which runs" style={{ padding: "0 20px" }}>
        {tabs.map(([k, label]) => (
          <button key={k} type="button" aria-pressed={cls === k} onClick={() => setCls(k)} title={k === "all" ? "Every CI run" : CLASS_HELP[k]}>
            {label}
            <span className="n">{formatCount(counts[k])}</span>
          </button>
        ))}
      </div>
      <div className="plot" role="img" aria-label={has ? `Test time per CI run, ${cls === "all" ? "all runs" : CLASS_LABEL[cls]}: ${summaryLine(m, cls)}` : "No CI runs of this kind in this period"}>
        {has ? (
          <>
            <span className="plot-max tnum">{formatDuration(max / 1000 / 1.1)}</span>
            <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" aria-hidden="true">
              <line x1="0" x2={W} y1={H - 0.5} y2={H - 0.5} className="plot-axis" />
              {adoptIdx >= 0 ? <line x1={xs(adoptIdx)} x2={xs(adoptIdx)} y1="0" y2={H} className="plot-mark" /> : null}
              <path d={path(withoutPts)} className="plot-without" />
              <path d={path(withPts)} className="plot-with" />
            </svg>
            {withPts.map((pt, i) => (
              <span key={`w${i}`} className="plot-dot" style={{ left: `${(pt.x / W) * 100}%`, top: `${(pt.y / H) * 100}%` }} aria-hidden="true" />
            ))}
            {adoptIdx >= 0 ? (
              <span className="plot-mark-label" style={{ left: `${(xs(adoptIdx) / W) * 100}%` }}>
                Quod added
              </span>
            ) : null}
            <ol className="plot-hits" aria-hidden="true">
              {days.map((d, i) =>
                d.with[cls] === null ? (
                  <li key={d.day} />
                ) : (
                  <li key={d.day} className={i < n / 3 ? "edge-l" : i >= (2 * n) / 3 ? "edge-r" : ""}>
                    <span className="tip">
                      <strong>{formatDay(d.day)}</strong>
                      <br />
                      With Quod {formatDuration(d.with[cls]! / 1000)} · without ≈ {formatDuration(d.without[cls]! / 1000)}
                    </span>
                  </li>
                ),
              )}
            </ol>
          </>
        ) : (
          <p className="muted plot-empty">No {cls === "all" ? "CI runs" : CLASS_LABEL[cls].toLowerCase()} with tests in this period.</p>
        )}
      </div>
      <div className="axis" aria-hidden="true" style={{ padding: "0 20px" }}>
        <span>{days[0] ? formatDay(days[0].day) : ""}</span>
        <span>Today</span>
      </div>
      <table className="class-table">
        <caption className="sr-only">Typical test time per CI run, by kind of run</caption>
        <thead>
          <tr>
            <th scope="col">Runs</th>
            <th scope="col">Without Quod (est.)</th>
            <th scope="col">With Quod</th>
            <th scope="col">Change</th>
          </tr>
        </thead>
        <tbody>
          {RUN_CLASSES.map((k) => {
            const c = m.byClass[k];
            return (
              <tr key={k} aria-current={cls === k ? "true" : undefined}>
                <th scope="row">
                  <button type="button" className="link-btn" onClick={() => setCls(cls === k ? "all" : k)} title={CLASS_HELP[k]}>
                    {CLASS_LABEL[k]}
                  </button>
                  <span className="faint"> · {formatCount(c.ciRuns)}</span>
                </th>
                <td className="tnum">{c.typicalWithoutMs === null ? "–" : formatDuration(c.typicalWithoutMs / 1000)}</td>
                <td className="tnum">{c.typicalWithMs === null ? "–" : formatDuration(c.typicalWithMs / 1000)}</td>
                <td className="tnum change">{changeText(c.typicalWithMs, c.typicalWithoutMs) ?? "–"}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
      <p className="faint chart-foot">
        Typical means the median CI run. A CI run&apos;s test time adds up all its test suites; checkout and install are not included.
      </p>
    </section>
  );
}

function summaryLine(m: SavingsModel, cls: ClassFilter): string {
  const pick = cls === "all" ? null : m.byClass[cls];
  if (pick) {
    const ch = changeText(pick.typicalWithMs, pick.typicalWithoutMs);
    return pick.typicalWithMs === null ? "no runs" : `typically ${formatDuration(pick.typicalWithMs / 1000)} with Quod, about ${formatDuration((pick.typicalWithoutMs ?? 0) / 1000)} without${ch ? ` (${ch})` : ""}`;
  }
  return `${formatCount(m.ciRuns)} CI runs`;
}

function MoneyCard({ m }: { m: SavingsModel }) {
  const days = m.days;
  const n = days.length;
  const xs = (i: number) => (n <= 1 ? W / 2 : (i / (n - 1)) * W);
  const vals = days.map((d) => d.cumulativeUsd);
  const top = Math.max(0, ...vals);
  const bottom = Math.min(0, ...vals);
  const span = Math.max(top - bottom, 0.01);
  const ys = (v: number) => H - ((v - bottom) / span) * H * 0.92 - H * 0.04;
  const pts = days.map((d, i) => ({ x: xs(i), y: ys(d.cumulativeUsd) }));
  const zero = ys(0);
  const last = days[n - 1]?.cumulativeUsd ?? 0;
  const area = pts.length ? `${path(pts)}L${W},${zero}L0,${zero}Z` : "";
  return (
    <section className="card chart-card" aria-labelledby="money-h">
      <div className="chart-card-head">
        <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          <h2 id="money-h" className="h2">
            Money saved over time
          </h2>
          <p className="faint" style={{ fontSize: 12, lineHeight: 1.5 }}>
            Added up day by day, after background recordings. At {rateText(m.usdPerMinute)}.
          </p>
        </div>
        <span className={`money-last tnum ${last < 0 ? "neg" : ""}`}>{formatMoney(last)}</span>
      </div>
      <div className="plot" role="img" aria-label={`Money saved, added up over the period: ${formatMoney(last)} by today`}>
        <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" aria-hidden="true">
          <line x1="0" x2={W} y1={zero} y2={zero} className="plot-axis" />
          {area ? <path d={area} className={`plot-area ${last < 0 ? "neg" : ""}`} /> : null}
          {pts.length ? <path d={path(pts)} className={`plot-money ${last < 0 ? "neg" : ""}`} /> : null}
        </svg>
        <ol className="plot-hits" aria-hidden="true">
          {days.map((d, i) => (
            <li key={d.day} className={i < n / 3 ? "edge-l" : i >= (2 * n) / 3 ? "edge-r" : ""}>
              <span className="tip">
                <strong>{formatDay(d.day)}</strong>
                <br />
                {formatMoney(d.cumulativeUsd)} so far
              </span>
            </li>
          ))}
        </ol>
      </div>
      <div className="axis" aria-hidden="true" style={{ padding: "0 20px" }}>
        <span>{days[0] ? formatDay(days[0].day) : ""}</span>
        <span>Today</span>
      </div>
      <details className="data-table" style={{ padding: "0 20px 16px" }}>
        <summary>Show the numbers</summary>
        <div className="scroll">
          <table>
            <caption className="sr-only">Test time and money saved per day</caption>
            <thead>
              <tr>
                <th scope="col">Day</th>
                <th scope="col">Typical run, with Quod</th>
                <th scope="col">Without (est.)</th>
                <th scope="col">Saved so far</th>
              </tr>
            </thead>
            <tbody>
              {[...days].reverse().map((d) => (
                <tr key={d.day}>
                  <td>{formatDay(d.day)}</td>
                  <td>{d.with.all === null ? "–" : formatDuration(d.with.all / 1000)}</td>
                  <td>{d.without.all === null ? "–" : formatDuration(d.without.all / 1000)}</td>
                  <td>{formatMoney(d.cumulativeUsd)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </details>
    </section>
  );
}

/** "Now running" folded into one line (the count stays live); open it to see each job. */
export function NowRunningFold({ children, initialCount }: { children: React.ReactNode; initialCount: number }) {
  const live = useLive();
  const jobs = live.jobs;
  const running = jobs ? jobs.filter((j) => j.status === "in_progress").length : initialCount;
  const queued = jobs ? jobs.filter((j) => j.status === "queued").length : 0;
  return (
    <details className="card fold">
      <summary>
        <span className="fold-caret" aria-hidden="true">
          ›
        </span>
        <span style={{ fontWeight: 600, fontSize: 15 }}>Now running</span>
        <span className="faint" style={{ fontSize: 13 }}>
          {running === 0 && queued === 0 ? "Nothing right now" : `${formatCount(running)} running${queued ? `, ${formatCount(queued)} queued` : ""}`}
        </span>
        {running > 0 ? <span className="dot blue pulse sm" aria-hidden="true" /> : null}
      </summary>
      <div className="fold-body">{children}</div>
    </details>
  );
}

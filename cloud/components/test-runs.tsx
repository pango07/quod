// Test runs (test-level replay): the Overview card and the list of runs.
// Words come from lib/test-runs.ts.

import Link from "next/link";
import { Chip, Dot } from "@/components/ui";
import type { TestRunSummary, TestStats } from "@/lib/api/types";
import { formatAgo, formatCount, shortSha } from "@/lib/format";
import { exitLabel, formatSaved, kindLabel, KIND_EXPLAINED, SAVED_EXPLAINED, suiteLabel, testRunHref, weekSkipped } from "@/lib/test-runs";

/** One line per run: headline numbers, branch and commit, when. */
export function TestRunsTable({ team, runs, asOf, showRepo }: { team: string; runs: TestRunSummary[]; asOf: string; showRepo?: boolean }) {
  return (
    <div>
      <div className="table-head cols-tests" aria-hidden="true">
        <span>Tests</span>
        <span>Skipped (unchanged)</span>
        <span>Ran</span>
        <span title={SAVED_EXPLAINED}>Saved</span>
        <span>When</span>
      </div>
      {runs.map((r) => {
        const exit = exitLabel(r.exit);
        return (
          <Link key={r.id} className="trow cols-tests" href={testRunHref(team, r)}>
            <span className="cell-main">
              <span style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <Dot tone={exit.tone} />
                <span className="sr-only">{exit.text}, </span>
                <span className="ellipsis">
                  {r.tool} · {suiteLabel(r)}
                </span>
                <span title={KIND_EXPLAINED[r.kind]}>
                  <Chip tone={r.kind === "shadow" ? "gray" : "blue"}>{kindLabel(r.kind)}</Chip>
                </span>
              </span>
              <span className="mono cell-sub">
                {showRepo ? `${r.repoSlug} · ` : ""}
                {r.branch ?? "unknown branch"}
                {r.sha ? ` · ${shortSha(r.sha)}` : ""}
              </span>
            </span>
            <span className="mono" style={{ fontSize: 13 }}>
              <span className="label-sm" style={{ fontFamily: "var(--sans)" }}>
                Skipped (unchanged){" "}
              </span>
              {formatCount(r.files.replayed)} of {formatCount(r.files.total)}
            </span>
            <span className="mono" style={{ fontSize: 13 }}>
              <span className="label-sm" style={{ fontFamily: "var(--sans)" }}>
                Ran{" "}
              </span>
              {formatCount(r.files.ran)}
            </span>
            <span className="mono" style={{ fontSize: 13 }} title={SAVED_EXPLAINED}>
              <span className="label-sm" style={{ fontFamily: "var(--sans)" }}>
                Saved{" "}
              </span>
              {formatSaved(r.timing.savedMs)}
            </span>
            <span style={{ fontSize: 13, color: "var(--muted)" }}>
              <span className="label-sm">When </span>
              <time dateTime={r.at}>{formatAgo(r.at, asOf)}</time>
            </span>
          </Link>
        );
      })}
    </div>
  );
}

/** "280 test files: 258 skipping, 17 ready, 3 always run" with a link to Trust, where each file's state is listed. */
export function TestFilesLine({ team, line }: { team: string; line: string }) {
  return (
    <p style={{ fontSize: 14, display: "flex", gap: 8, flexWrap: "wrap", alignItems: "baseline" }}>
      <span>{line}</span>
      <Link href={`/${team}/trust`} className="link-muted" style={{ fontSize: 13 }}>
        See each file
      </Link>
    </p>
  );
}

/** The Overview's test card: time saved today and this week, skips, wrong results, spot-checks, and the latest runs. */
export function TestStatsCard({
  team,
  stats,
  filesLine,
  spotChecksWeek,
}: {
  team: string;
  stats: TestStats;
  filesLine?: string | null;
  /** Spot-checks over the last 7 days (from /test-runs); without it, the stats' all-time count, labelled so. */
  spotChecksWeek?: { checked: number; mismatched: number } | null;
}) {
  const skipped = weekSkipped(stats);
  const wrong = stats.wrongResults;
  const recent = stats.recent.slice(0, 5);
  const rows: [string, string, string?][] = [
    ["Time saved today", formatSaved(stats.today.savedMs), SAVED_EXPLAINED],
    ["Time saved this week", formatSaved(stats.week.savedMs), SAVED_EXPLAINED],
    [
      "Test files skipped this week",
      skipped ? `${skipped} (${formatCount(stats.week.filesSkipped)} of ${formatCount(stats.week.filesTotal)})` : "None yet",
      "Skipped because nothing they depend on changed since a recorded run; their earlier result was reused.",
    ],
    ["Wrong results", formatCount(wrong), "Spot-checks that found a skipped file would have given a different result if it had run. This should be 0."],
    spotChecksWeek
      ? [
          "Spot-checks passed, last 7 days",
          spotChecksWeek.checked ? `${formatCount(spotChecksWeek.checked - spotChecksWeek.mismatched)} of ${formatCount(spotChecksWeek.checked)}` : "None this week",
          "A few skipped files are re-run in the background and their results compared.",
        ]
      : [
          "Spot-checks passed, all time",
          stats.audits.checked ? `${formatCount(stats.audits.passed)} of ${formatCount(stats.audits.checked)}` : "None yet",
          "A few skipped files are re-run in the background and their results compared.",
        ],
  ];
  return (
    <section className="card" aria-labelledby="tests-h" style={{ overflow: "hidden" }}>
      <div className="card-head">
        <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          <h2 id="tests-h" className="h2">
            Test files skipped when unchanged
          </h2>
          <p className="muted" style={{ fontSize: 13, lineHeight: 1.5, maxWidth: "72ch" }}>
            Quod skips a test file when nothing it depends on changed since a recorded run, and reuses that result. A few skipped files are re-run in the background to
            check.
          </p>
        </div>
      </div>
      {filesLine ? (
        <div style={{ padding: "0 20px 12px" }}>
          <TestFilesLine team={team} line={filesLine} />
        </div>
      ) : null}
      <dl className="stats sm" style={{ padding: "0 20px 16px", margin: 0 }}>
        {rows.map(([k, v, why]) => (
          <div key={k} className="card stat" title={why}>
            <dt className="label">{k}</dt>
            <dd className="value" style={{ margin: 0, fontSize: 20, ...(k === "Wrong results" && wrong > 0 ? { color: "var(--red)" } : {}) }}>
              {v}
            </dd>
          </div>
        ))}
      </dl>
      {wrong > 0 ? (
        <p className="notice" style={{ margin: "0 20px 16px" }}>
          <Dot tone="amber" />
          <span>
            {wrong === 1 ? "1 skipped test file" : `${formatCount(wrong)} skipped test files`} would have given a different result if it had run. Open the runs below to see which.
          </span>
        </p>
      ) : null}
      <p className="faint" style={{ fontSize: 12, padding: "0 20px 12px" }}>
        {SAVED_EXPLAINED}
      </p>
      {recent.length ? (
        <>
          <h3 className="sr-only">Recent test runs</h3>
          <TestRunsTable team={team} runs={recent} asOf={stats.asOf} showRepo />
        </>
      ) : null}
    </section>
  );
}

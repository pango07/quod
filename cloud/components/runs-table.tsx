import Link from "next/link";
import type { RunSummary } from "@/lib/api/types";
import { formatAgo, formatDuration, shortSha } from "@/lib/format";
import { Dot } from "@/components/ui";

export function RunsTable({ team, runs, asOf, showRepo }: { team: string; runs: RunSummary[]; asOf: string; showRepo?: boolean }) {
  return (
    <div>
      <div className="table-head cols-runs" aria-hidden="true">
        <span>Run</span>
        <span>Steps replayed</span>
        <span>Took</span>
        <span>Instead of</span>
        <span>Started</span>
      </div>
      {runs.map((r) => (
        <Link key={`${r.repoSlug}/${r.number}`} className="trow cols-runs" href={`/${team}/${r.repoSlug}/runs/${r.number}`}>
          <span className="cell-main">
            <span style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <Dot tone={r.status === "passed" ? "green" : r.status === "failed" ? "red" : "blue"} />
              <span className="sr-only">{r.status}, </span>
              <span className="ellipsis">{r.title}</span>
            </span>
            <span className="mono cell-sub">
              {showRepo ? `${r.repoSlug} · ` : ""}#{r.number} · {r.branch} · {shortSha(r.sha)}
            </span>
          </span>
          <span className="mono" style={{ fontSize: 13 }}>
            <span className="label-sm" style={{ fontFamily: "var(--sans)" }}>
              Steps replayed{" "}
            </span>
            {r.stepsReplayed} of {r.stepsTotal}
          </span>
          <span className="mono" style={{ fontSize: 13 }}>
            <span className="label-sm" style={{ fontFamily: "var(--sans)" }}>
              Took{" "}
            </span>
            {formatDuration(r.tookSeconds)}
          </span>
          <span className="mono" style={{ fontSize: 13, color: "var(--muted)" }}>
            <span className="label-sm" style={{ fontFamily: "var(--sans)" }}>
              Instead of{" "}
            </span>
            {formatDuration(r.wouldHaveTakenSeconds)}
          </span>
          <span style={{ fontSize: 13, color: "var(--muted)" }}>
            <span className="label-sm">Started </span>
            <time dateTime={r.startedAt}>{formatAgo(r.startedAt, asOf)}</time>
          </span>
        </Link>
      ))}
    </div>
  );
}

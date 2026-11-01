"use client";

import Link from "next/link";
import { useState } from "react";
import { TrustChip } from "@/components/ui";
import type { StepSummary } from "@/lib/api/types";
import { formatDuration, formatMinutes, formatPercent } from "@/lib/format";

import type { StepFilter as Filter } from "@/lib/params";

const FILTERS: [Filter, string][] = [
  ["all", "All"],
  ["replaying", "Replaying"],
  ["proving", "Proving"],
  ["watching", "Watching"],
  ["paused", "Paused"],
];

/** The steps of a repo (or a team) with state filters. The filter is kept in the URL (?state=). */
export function StepsTable({ team, steps, initial, showRepo }: { team: string; steps: StepSummary[]; initial: Filter; showRepo?: boolean }) {
  const [filter, setFilter] = useState<Filter>(initial);
  const present = new Set(steps.map((s) => s.trust.kind));
  const filters = FILTERS.filter(([k]) => k === "all" || k !== "paused" || present.has("paused"));
  const rows = filter === "all" ? steps : steps.filter((s) => s.trust.kind === filter);

  const pick = (f: Filter) => {
    setFilter(f);
    const url = new URL(window.location.href);
    if (f === "all") url.searchParams.delete("state");
    else url.searchParams.set("state", f);
    window.history.replaceState(null, "", url);
  };

  return (
    <section className="card" aria-labelledby="steps-h" style={{ overflow: "hidden" }}>
      <div className="card-head">
        <h2 id="steps-h" className="h2">
          Steps
        </h2>
        <div className="pill-filters" role="group" aria-label="Filter steps">
          {filters.map(([k, label]) => (
            <button key={k} type="button" aria-pressed={filter === k} onClick={() => pick(k)}>
              {label}
            </button>
          ))}
        </div>
      </div>
      <div className="table-head cols-steps" aria-hidden="true">
        <span>Job / step</span>
        <span>State</span>
        <span>Replayed</span>
        <span>Ran vs replayed</span>
        <span>Saved</span>
      </div>
      <p className="sr-only" aria-live="polite">
        {rows.length === 1 ? "1 step" : `${rows.length} steps`} shown
      </p>
      {rows.length === 0 ? (
        <p className="trow muted" style={{ display: "block", fontSize: 13 }}>
          No steps are {filter} right now.
        </p>
      ) : null}
      {rows.map((s) => (
        <Link key={`${s.repoSlug}/${s.key}`} className="trow cols-steps" href={`/${team}/${s.repoSlug}/steps/${s.key}`}>
          <span className="cell-main">
            <span className="mono" style={{ fontSize: 13 }}>
              {s.command}
            </span>
            <span className="cell-sub">{showRepo ? `${s.repoSlug} · ${s.job}` : s.job}</span>
          </span>
          <span>
            <span className="sr-only">State: </span>
            <TrustChip trust={s.trust} />
          </span>
          <span className="mono" style={{ fontSize: 13 }}>
            <span className="label-sm" style={{ fontFamily: "var(--sans)" }}>
              Replayed{" "}
            </span>
            {s.replayRate === null ? "n/a" : formatPercent(s.replayRate)}
          </span>
          <span className="mono" style={{ fontSize: 13, color: "var(--muted)" }}>
            <span className="label-sm" style={{ fontFamily: "var(--sans)" }}>
              Ran vs replayed{" "}
            </span>
            {s.replaySeconds === null ? formatDuration(s.ranSeconds) : `${formatDuration(s.ranSeconds)} → ${formatDuration(s.replaySeconds)}`}
          </span>
          <span className="mono" style={{ fontSize: 13 }}>
            <span className="label-sm" style={{ fontFamily: "var(--sans)" }}>
              Saved{" "}
            </span>
            {formatMinutes(s.savedMinutes)}
          </span>
        </Link>
      ))}
    </section>
  );
}

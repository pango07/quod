"use client";

// The plain-wording Trust list (cloud/trust-plain; lib/ui-version.ts picks it).
// One row per job and command; earlier versions of the step fold under it.
// The classic list is components/steps-table.tsx, unchanged.

import Link from "next/link";
import { useEffect, useRef, useState } from "react";
import { Dot } from "@/components/ui";
import { formatAgo } from "@/lib/format";
import { LEGEND, lastResult, savedText, skippedText, type PlainKind, type PlainState, type StepGroup, type Version } from "@/lib/trust-plain";

type Filter = "all" | PlainKind;
const FILTERS: [Filter, string][] = [
  ["all", "All"],
  ["learning", "Learning"],
  ["ready", "Ready to skip"],
  ["skipping", "Skipping"],
  ["paused", "Paused"],
  ["recording", "Recording only"],
  ["always", "Always runs"],
];

export function PlainStateChip({ state, href }: { state: PlainState; href?: string }) {
  const tone = state.tone;
  const chip = (
    <span className={`chip wrap ${tone === "blue" || tone === "amber" ? tone : ""}`} title={state.help}>
      <Dot tone={tone === "gray" ? "white" : tone} size="sm" />
      {state.label}
    </span>
  );
  return href ? (
    <Link href={href} title={state.help} style={{ color: "inherit" }}>
      {chip}
    </Link>
  ) : (
    chip
  );
}

function VersionCells({ v, team, asOf }: { v: Version; team: string; asOf: string }) {
  const href = `/${team}/${v.step.repoSlug}/steps/${v.step.key}`;
  return (
    <>
      <span>
        <span className="sr-only">State: </span>
        <PlainStateChip state={v.state} href={v.state.kind === "paused" ? `${href}#diff` : undefined} />
        {v.step.trustRef ? (
          <span className="cell-sub" style={{ display: "block", marginTop: 4 }} title="The default branch has not recorded this step yet; this is what runs on that branch do.">
            on {v.step.trustRef}
          </span>
        ) : null}
      </span>
      <span style={{ fontSize: 13 }}>
        <span className="label-sm">Last result </span>
        {lastResult(v, asOf, formatAgo)}
      </span>
      <span style={{ fontSize: 13 }}>
        <span className="label-sm">Skipped </span>
        {skippedText(v)}
      </span>
    </>
  );
}

function GroupRow({ g, team, asOf, showRepo }: { g: StepGroup; team: string; asOf: string; showRepo?: boolean }) {
  const [open, setOpen] = useState(false);
  const v = g.current;
  const href = `/${team}/${v.step.repoSlug}/steps/${v.step.key}`;
  const listId = `v-${g.current.step.key.slice(0, 12)}`;
  return (
    <div className="plain-group">
      <div className="trow cols-plain">
        <span className="cell-main">
          <Link href={href} className={g.untitled ? "" : "mono"} style={{ fontSize: 13, color: "var(--text)" }} title="Open this step's history">
            {g.title}
          </Link>
          <span className="cell-sub">
            {[showRepo ? g.repoSlug : null, g.job || null].filter(Boolean).join(" · ") || " "}
            {g.earlier.length ? (
              <>
                {" · "}
                <button type="button" className="link-btn" aria-expanded={open} aria-controls={listId} onClick={() => setOpen((o) => !o)}>
                  {open ? "Hide" : ""} {g.earlier.length === 1 ? "1 earlier version" : `${g.earlier.length} earlier versions`}
                </button>
              </>
            ) : null}
          </span>
        </span>
        <VersionCells v={v} team={team} asOf={asOf} />
        <span className="mono" style={{ fontSize: 13 }}>
          <span className="label-sm" style={{ fontFamily: "var(--sans)" }}>
            Time saved{" "}
          </span>
          {savedText(g.savedSeconds)}
        </span>
      </div>
      {open ? (
        <div id={listId} className="plain-versions">
          <p className="cell-sub" style={{ padding: "8px 20px 4px 36px", lineHeight: 1.5 }}>
            Why there are versions: {g.reason}
          </p>
          {g.earlier.map((e, i) => (
            <div key={e.step.key} className="trow cols-plain plain-version">
              <span className="cell-main">
                <Link href={`/${team}/${e.step.repoSlug}/steps/${e.step.key}`} style={{ fontSize: 13 }}>
                  Version {g.earlier.length - i}
                  <span className="mono faint"> · {e.step.key.slice(0, 8)}</span>
                </Link>
                <span className="cell-sub">{e.seen ? "superseded" : "not in recent runs"}</span>
              </span>
              <VersionCells v={e} team={team} asOf={asOf} />
              <span className="mono faint" style={{ fontSize: 13 }}>
                {savedText(Math.max(e.step.savedMinutes * 60, e.seen?.savedTotal ?? 0))}
              </span>
            </div>
          ))}
        </div>
      ) : null}
    </div>
  );
}

/** Trust (and a repo's Steps tab), plain: grouped rows, states in words, a legend. */
export function TrustList({ team, groups, asOf, showRepo, recentRuns }: { team: string; groups: StepGroup[]; asOf: string; showRepo?: boolean; recentRuns: number }) {
  const [filter, setFilter] = useState<Filter>("all");
  const present = new Set(groups.map((g) => g.current.state.kind));
  const filters = FILTERS.filter(([k]) => k === "all" || present.has(k as PlainKind));
  const rows = filter === "all" ? groups : groups.filter((g) => g.current.state.kind === filter);
  return (
    <section className="card" aria-labelledby="steps-h" style={{ overflow: "hidden" }}>
      <div className="card-head">
        <h2 id="steps-h" className="h2">
          Steps
        </h2>
        {filters.length > 2 ? (
          <div className="pill-filters" role="group" aria-label="Filter steps">
            {filters.map(([k, label]) => (
              <button key={k} type="button" aria-pressed={filter === k} onClick={() => setFilter(k)}>
                {label}
              </button>
            ))}
          </div>
        ) : null}
      </div>
      <div className="table-head cols-plain" aria-hidden="true">
        <span>Step</span>
        <span>State</span>
        <span>Last result</span>
        <span title={`Counted over the last ${recentRuns} runs`}>Skipped</span>
        <span>Time saved</span>
      </div>
      <p className="sr-only" aria-live="polite">
        {rows.length === 1 ? "1 step" : `${rows.length} steps`} shown
      </p>
      {rows.length === 0 ? (
        <p className="trow muted" style={{ display: "block", fontSize: 13 }}>
          No steps here right now.
        </p>
      ) : null}
      {rows.map((g) => (
        <GroupRow key={g.id} g={g} team={team} asOf={asOf} showRepo={showRepo} />
      ))}
      <Legend />
    </section>
  );
}

export function Legend() {
  return (
    <details className="plain-legend">
      <summary>What the states mean</summary>
      <dl>
        {LEGEND.map((l) => (
          <div key={l.kind}>
            <dt>{l.label}</dt>
            <dd>{l.help}</dd>
          </div>
        ))}
        <div>
          <dt>Versions</dt>
          <dd>
            Quod keys a step by its command and everything that can change its result: the environment, tool versions, and Quod itself. When one of those changes, the step
            starts a new version, which learns again. Earlier versions fold under the current one.
          </dd>
        </div>
      </dl>
    </details>
  );
}

/** The writer-trust explanation as one line; "Learn more" opens it (and /trust#protected-branch opens it too). */
export function ProtectionNote({ children }: { children: React.ReactNode }) {
  const ref = useRef<HTMLDetailsElement>(null);
  useEffect(() => {
    if (window.location.hash === "#protected-branch" && ref.current) ref.current.open = true;
  }, []);
  return (
    <details ref={ref} id="protected-branch" className="plain-note" style={{ scrollMarginTop: 80 }}>
      <summary>
        <span className="muted">Only runs on a writer branch save the recordings that other runs reuse.</span> <span className="link-like">Learn more</span>
      </summary>
      <div style={{ paddingTop: 12 }}>{children}</div>
    </details>
  );
}

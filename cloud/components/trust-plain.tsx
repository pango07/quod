"use client";

// The plain-wording Trust list (cloud/trust-plain; lib/ui-version.ts picks it).
// One row per job and command; earlier versions of the step fold under it.
// The classic list is components/steps-table.tsx, unchanged.

import Link from "next/link";
import { useEffect, useRef, useState } from "react";
import { Dot } from "@/components/ui";
import { formatAgo } from "@/lib/format";
import { LEGEND, lastResult, savedLine, STATE_WORD, stepSentence, type PlainKind, type PlainState, type StepGroup } from "@/lib/trust-plain";

type Filter = "all" | PlainKind;
const FILTERS: [Filter, string][] = [
  ["all", "All"],
  ["learning", "Learning"],
  ["ready", "Ready to skip"],
  ["skipping", "Skipping"],
  ["paused", "Paused"],
  ["recording", "Watching only"],
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

function GroupRow({ g, team, asOf, recentRuns }: { g: StepGroup; team: string; asOf: string; recentRuns: number }) {
  const [open, setOpen] = useState(false);
  const v = g.current;
  const href = `/${team}/${v.step.repoSlug}/steps/${v.step.key}`;
  const listId = `v-${g.current.step.key.slice(0, 12)}`;
  const tone = v.state.tone;
  return (
    <li className="step-item">
      <div className="step-line1">
        <Link href={v.state.kind === "paused" ? `${href}#diff` : href} className={`step-title ${g.untitled ? "" : "mono"}`} title="Open this step's history">
          {g.title}
        </Link>
        <span className={`chip ${tone === "blue" || tone === "amber" ? tone : ""}`} title={v.state.help}>
          <Dot tone={tone === "gray" ? "white" : tone} size="sm" />
          {STATE_WORD[v.state.kind]}
        </span>
      </div>
      <p className="step-says">{stepSentence(v, recentRuns)}</p>
      <p className="step-meta">
        {g.job ? <span>Job: {g.job}</span> : null}
        {v.step.trustRef ? <span title="The default branch has not recorded this step yet; this is what runs on that branch do.">On {v.step.trustRef}</span> : null}
        <span>
          {v.seen && (v.seen.outcome === "ran" || v.seen.outcome === "replayed") ? "Last run: " : ""}
          {lastResult(v, asOf, formatAgo)}
        </span>
        {g.savedSeconds >= 1 || g.couldSaveSeconds < 1 ? <span className={g.savedSeconds >= 1 ? "step-saved" : ""}>{savedLine("skipping", g.savedSeconds)}</span> : null}
        {g.couldSaveSeconds >= 1 ? <span>{savedLine("recording", g.couldSaveSeconds)}</span> : null}
        {g.earlier.length ? (
          <button type="button" className="link-btn" aria-expanded={open} aria-controls={listId} onClick={() => setOpen((o) => !o)}>
            {open ? "Hide earlier versions" : g.earlier.length === 1 ? "1 earlier version" : `${g.earlier.length} earlier versions`}
          </button>
        ) : null}
      </p>
      {open ? (
        <div id={listId} className="step-versions">
          <p className="faint" style={{ fontSize: 12, lineHeight: 1.5 }}>
            {g.reason}
          </p>
          <ul>
            {g.earlier.map((e, i) => (
              <li key={e.step.key}>
                <Link href={`/${team}/${e.step.repoSlug}/steps/${e.step.key}`}>
                  Version {g.earlier.length - i}
                  <span className="mono faint"> · {e.step.key.slice(0, 8)}</span>
                </Link>
                <span className="faint">
                  {STATE_WORD[e.state.kind]} · {e.seen ? "replaced by a newer version" : "not in recent runs"} · {savedLine(e.state.kind, Math.max(e.step.savedMinutes * 60, e.seen?.savedTotal ?? 0))}
                </span>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </li>
  );
}

/** Trust (and a repo's Steps tab), plain: one entry per step that says in words what Quod does with it, grouped by repository. */
export function TrustList({ team, groups, asOf, showRepo, recentRuns }: { team: string; groups: StepGroup[]; asOf: string; showRepo?: boolean; recentRuns: number }) {
  const [filter, setFilter] = useState<Filter>("all");
  const present = new Set(groups.map((g) => g.current.state.kind));
  const filters = FILTERS.filter(([k]) => k === "all" || present.has(k as PlainKind));
  const rows = filter === "all" ? groups : groups.filter((g) => g.current.state.kind === filter);
  const repos = showRepo ? [...new Set(rows.map((g) => g.repoSlug))] : [null];
  return (
    <section className="card" aria-labelledby="steps-h" style={{ overflow: "hidden" }}>
      <div className="card-head">
        <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          <h2 id="steps-h" className="h2">
            CI steps
          </h2>
          <p className="faint" style={{ fontSize: 12, lineHeight: 1.5, maxWidth: "72ch" }}>
            Whole commands in your workflows, like a lint or a build. Quod skips one when nothing it reads has changed since a run it trusts.
          </p>
        </div>
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
      <p className="sr-only" aria-live="polite">
        {rows.length === 1 ? "1 step" : `${rows.length} steps`} shown
      </p>
      {rows.length === 0 ? (
        <p className="trow muted" style={{ display: "block", fontSize: 13 }}>
          No steps here right now.
        </p>
      ) : null}
      {repos.map((repo) => (
        <div key={repo ?? "all"}>
          {repo ? <h3 className="step-repo">{repo}</h3> : null}
          <ul className="step-list">
            {rows
              .filter((g) => repo === null || g.repoSlug === repo)
              .map((g) => (
                <GroupRow key={g.id} g={g} team={team} asOf={asOf} recentRuns={recentRuns} />
              ))}
          </ul>
        </div>
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

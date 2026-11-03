"use client";

// Trust, test-first and compact: a summary strip, the files that need a look
// (paused, always runs), then one collapsed row per suite. A sticky toolbar
// filters the files by path and state. The numbers are worked out on the
// server (trustModel in lib/test-suites.ts); this only filters and expands.

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useMemo, useState, useTransition } from "react";
import { EmptyState } from "@/components/ui";
import type { TestFile, TestFileState } from "@/lib/api/types";
import { formatAgo, formatCount } from "@/lib/format";
import { formatSaved, LAST_CHECKS, SAVED_EXPLAINED, shareText, shareTitle } from "@/lib/test-runs";
import {
  type BarSegment,
  FILE_LEGEND,
  FILTERS,
  filterCounts,
  lastResultText,
  matchesFilter,
  MAX_FILES,
  type StateFilter,
  type SuiteRow,
  type TrustModel,
  visibleFiles,
} from "@/lib/test-suites";

const LEGEND = Object.fromEntries(FILE_LEGEND.map((l) => [l.state, l])) as Record<TestFileState, (typeof FILE_LEGEND)[number]>;
const stateWord = (s: TestFileState) => LEGEND[s]?.label ?? String(s);

function StateBar({ segments, small, label }: { segments: BarSegment[]; small?: boolean; label: string }) {
  return (
    <span className={`statebar ${small ? "sm" : ""}`} role="img" aria-label={label}>
      {segments.map((s) => (
        <span key={s.state} className={`st-${s.state}`} style={{ flex: `${s.n} 1 0` }} title={`${formatCount(s.n)} ${s.label}`} />
      ))}
    </span>
  );
}

const barLabel = (segments: BarSegment[]) => segments.map((s) => `${formatCount(s.n)} ${s.label}`).join(", ") || "No test files";

function Summary({ s }: { s: TrustModel["summary"] }) {
  return (
    <section aria-label="Summary" className="tr-strip">
      <div>
        <span className="label">Wrong results</span>
        <span className="value tnum">{formatCount(s.wrongResults)}</span>
        <span className="note">
          {formatCount(s.spotChecksPassed)} spot-check{s.spotChecksPassed === 1 ? "" : "s"} passed, {s.spotChecksWindow === "week" ? "last 7 days" : "all time"}
        </span>
      </div>
      <div>
        <span className="label" title={SAVED_EXPLAINED}>
          Time saved, 7 days
        </span>
        <span className="value tnum">{formatSaved(s.savedWeekMs)}</span>
        <span className="note">
          across {formatCount(s.weekRuns)} run{s.weekRuns === 1 ? "" : "s"}
        </span>
      </div>
      <div>
        <span className="label">Skipped</span>
        <span className="value tnum" title={shareTitle(s.skipped)}>
          {shareText(s.skipped)}
        </span>
        <span className="note" title={shareTitle(s.skipped)}>
          of test files skipped, last {LAST_CHECKS} checks
        </span>
      </div>
      <div style={{ gap: 8 }}>
        <span className="label">{formatCount(s.totals.files)} test files</span>
        <StateBar segments={s.bar} label={barLabel(s.bar)} />
        <span className="tr-legend">
          {s.bar.map((b) => (
            <span key={b.state}>
              <span className={`sw st-${b.state}`} aria-hidden="true" />
              {formatCount(b.n)} {b.label}
            </span>
          ))}
        </span>
      </div>
    </section>
  );
}

function StateChip({ state }: { state: TestFileState }) {
  return (
    <span className={`chip ${state === "paused" ? "amber" : ""}`} title={LEGEND[state]?.help}>
      {stateWord(state)}
    </span>
  );
}

function NeedsLook({ items, filter, query }: { items: TrustModel["look"]; filter: StateFilter; query: string }) {
  if (items.length === 0) return <p className="muted">Nothing needs a look.</p>;
  const shown = items.filter((i) => matchesFilter(i, filter, query));
  return (
    <section className="card" aria-labelledby="look-h" style={{ overflow: "hidden" }}>
      <div className="card-head" style={{ padding: "12px 20px" }}>
        <h2 id="look-h" className="h2" style={{ fontSize: 14 }}>
          Needs a look <span className="faint" style={{ fontWeight: 400 }}>· {formatCount(items.length)}</span>
        </h2>
        <span className="faint" style={{ fontSize: 12 }}>
          Everything else is skipping or learning as it should
        </span>
      </div>
      {shown.length === 0 ? (
        <p className="look-row muted">None of these match the filter.</p>
      ) : (
        shown.map((i) => (
          <div key={`${i.suite}:${i.file}`} className="look-row">
            <StateChip state={i.state} />
            <span className="file">{i.file}</span>
            <span className="faint" style={{ fontSize: 12 }}>
              {i.suite}
            </span>
            <span className="why">{i.reason}</span>
            <Link href={i.href} className="link-muted">
              {i.linkText}
            </Link>
          </div>
        ))
      )}
    </section>
  );
}

function FileLine({ f }: { f: TestFile }) {
  const help = LEGEND[f.state]?.help ?? "";
  return (
    <div className="file-grid">
      <span className="mono ellipsis" style={{ fontSize: 12.5 }} title={f.file}>
        {f.file}
      </span>
      <span style={{ display: "inline-flex", alignItems: "center", gap: 6, minWidth: 0 }} title={f.reason ? `${help} (${f.reason})` : help}>
        <span className={`sw st-${f.state}`} aria-hidden="true" />
        <span className="ellipsis">{f.state === "always" && f.reason ? `${stateWord(f.state)}: ${f.reason}` : stateWord(f.state)}</span>
      </span>
      <span className="muted">
        <span className="label-sm">Last result </span>
        <span style={f.lastResult === "fail" ? { color: "var(--red)" } : undefined}>{lastResultText(f.lastResult)}</span>
      </span>
      <span className="tnum">
        <span className="label-sm">Skipped </span>
        {f.skipped > 0 ? `${formatCount(f.skipped)}×` : "0"}
      </span>
      <span className="tnum" title={SAVED_EXPLAINED}>
        <span className="label-sm">Saved </span>
        {formatSaved(f.savedMs ?? 0)}
      </span>
    </div>
  );
}

function Suite({ s, asOf, files, active, open, onToggle, showAll, onShowAll }: { s: SuiteRow; asOf: string; files: TestFile[]; active: boolean; open: boolean; onToggle: () => void; showAll: boolean; onShowAll: (all: boolean) => void }) {
  const id = `suite-${s.key.replace(/[^a-zA-Z0-9_-]/g, "-")}`;
  const { shown, hidden } = visibleFiles(files, showAll);
  return (
    <div>
      <button type="button" className="suite-row suite-grid" aria-expanded={open} aria-controls={id} onClick={onToggle}>
        <span style={{ display: "flex", flexDirection: "column", gap: 2, minWidth: 0 }}>
          <span style={{ fontWeight: 500, fontSize: 14 }} className="ellipsis">
            {s.heading}
          </span>
          {s.recorded ? null : (
            <span className="faint" style={{ fontSize: 12 }} title={s.ledger}>
              No recording yet
            </span>
          )}
        </span>
        <span style={{ display: "flex", flexDirection: "column", gap: 6, minWidth: 0 }}>
          <StateBar segments={s.bar} small label={barLabel(s.bar)} />
          <span className="faint" style={{ fontSize: 12 }}>
            {s.summary}
            {active ? ` · ${formatCount(files.length)} match` : ""}
          </span>
        </span>
        <span className="tnum" style={{ fontSize: 14 }} title={shareTitle(s.skipped)}>
          <span className="label-sm">Skipped </span>
          {shareText(s.skipped)}
        </span>
        <span className="muted">
          <span className="label-sm">Last run </span>
          {s.lastRunAt ? formatAgo(s.lastRunAt, asOf) : "No runs yet"}
        </span>
        <span className="tnum" style={{ fontSize: 14 }} title={SAVED_EXPLAINED}>
          <span className="label-sm">Saved, 7 days </span>
          {formatSaved(s.savedWeekMs)}
        </span>
        <span className="caret" aria-hidden="true">
          ›
        </span>
      </button>
      {open ? (
        <div id={id} className="suite-files">
          <p className="faint" style={{ fontSize: 12, padding: "4px 0 8px", display: "flex", gap: 12, flexWrap: "wrap", justifyContent: "space-between" }}>
            <span>{s.ledger}</span>
            <Link href={s.href} className="link-muted">
              Test runs
            </Link>
          </p>
          {files.length === 0 ? (
            <p className="muted" style={{ fontSize: 13, padding: "6px 0" }}>
              {s.files.length === 0 ? "No test files listed yet." : "No files here match the filter."}
            </p>
          ) : (
            <>
              <div className="file-grid head" aria-hidden="true">
                <span>File</span>
                <span>State</span>
                <span>Last result</span>
                <span>Skipped</span>
                <span title={SAVED_EXPLAINED}>Saved</span>
              </div>
              {shown.map((f) => (
                <FileLine key={`${f.project}:${f.file}`} f={f} />
              ))}
            </>
          )}
          {hidden > 0 || (showAll && files.length > MAX_FILES) ? (
            <div style={{ paddingTop: 10, display: "flex", gap: 12, alignItems: "center", flexWrap: "wrap" }}>
              <button type="button" className="btn" onClick={() => onShowAll(!showAll)}>
                {showAll ? `Show the first ${MAX_FILES}` : `Show all ${formatCount(files.length)} files`}
              </button>
              <span className="faint" style={{ fontSize: 12 }}>
                Needs a look first, then most time saved
              </span>
            </div>
          ) : null}
          {s.unlisted > 0 ? (
            <p className="faint" style={{ fontSize: 12, paddingTop: 8 }}>
              {formatCount(s.unlisted)} more not listed.
            </p>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

/** The compact Trust page body: summary, toolbar, needs a look, suites. */
export function TrustCompact({ model, asOf }: { model: TrustModel; asOf: string }) {
  const [filter, setFilter] = useState<StateFilter>("all");
  const [query, setQuery] = useState("");
  const [opened, setOpened] = useState<Record<string, boolean>>({});
  const [all, setAll] = useState<Record<string, boolean>>({});
  const allFiles = useMemo(() => model.suites.flatMap((s) => s.files), [model]);
  const counts = useMemo(() => filterCounts(allFiles, query), [allFiles, query]);
  const active = filter !== "all" || query.trim() !== "";
  const suites = model.suites.map((s) => ({ s, files: active ? s.files.filter((f) => matchesFilter(f, filter, query)) : s.files }));
  // While filtering, suites with no match step aside; a search opens the suites it matched (until closed by hand).
  const visible = active ? suites.filter((x) => x.files.length > 0) : suites;
  const searching = query.trim() !== "";
  return (
    <>
      <Summary s={model.summary} />
      <div className="tr-sections">
        <div className="tr-toolbar" role="search">
          <label className="tr-search">
            <svg aria-hidden="true" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <circle cx="11" cy="11" r="7" />
              <path d="M20 20l-3.5-3.5" />
            </svg>
            <span className="sr-only">Filter test files</span>
            <input type="search" placeholder="Filter files…" value={query} onChange={(e) => setQuery(e.target.value)} />
          </label>
          <div className="pill-filters" role="group" aria-label="State">
            {FILTERS.map((f) => (
              <button key={f.key} type="button" aria-pressed={filter === f.key} onClick={() => setFilter(f.key)}>
                {f.label}
                <span className="n">{formatCount(counts[f.key])}</span>
              </button>
            ))}
          </div>
        </div>
        <NeedsLook items={model.look} filter={filter} query={query} />
        <section className="card" aria-labelledby="suites-h" style={{ overflow: "hidden" }}>
          <div className="card-head" style={{ padding: "12px 20px" }}>
            <h2 id="suites-h" className="h2" style={{ fontSize: 14 }}>
              Suites <span className="faint" style={{ fontWeight: 400 }}>· {formatCount(model.suites.length)}</span>
            </h2>
          </div>
          <div>
            <div className="suite-head suite-grid" aria-hidden="true">
              <span>Suite</span>
              <span>Files</span>
              <span title={`Share of test files skipped over the suite's last ${LAST_CHECKS} checks (files added up)`}>
                Skipped
              </span>
              <span>Last run</span>
              <span title={SAVED_EXPLAINED}>
                Saved, 7d
              </span>
              <span />
            </div>
            {visible.map(({ s, files }) => (
              <Suite
                key={s.key}
                s={s}
                asOf={asOf}
                files={files}
                active={active}
                open={opened[s.key] ?? searching}
                onToggle={() => setOpened((o) => ({ ...o, [s.key]: !(o[s.key] ?? searching) }))}
                showAll={!!all[s.key]}
                onShowAll={(v) => setAll((a) => ({ ...a, [s.key]: v }))}
              />
            ))}
            {visible.length === 0 ? (
              <p className="look-row muted">
                No test files match.
              </p>
            ) : null}
          </div>
        </section>
      </div>
    </>
  );
}

/**
 * The test files could not be read (server down, timeout, a proxy's 404, no
 * session): say so and offer a retry, never the empty state. Retry re-runs
 * the page's server reads (router.refresh, which bypasses the client cache).
 */
export function TestsLoadError({ detail }: { detail?: string }) {
  const router = useRouter();
  const [pending, start] = useTransition();
  return (
    <section className="card" role="alert">
      <EmptyState
        title="Couldn't load test files"
        action={
          <button type="button" className="btn" onClick={() => start(() => router.refresh())} disabled={pending} aria-busy={pending}>
            {pending ? "Retrying…" : "Retry"}
          </button>
        }
      >
        Quod&apos;s server didn&apos;t answer just now, so your test files can&apos;t be shown. Nothing was lost; try again in a moment.
        {detail ? (
          <span className="faint" style={{ display: "block", fontSize: 12, marginTop: 8 }}>
            {detail}
          </span>
        ) : null}
      </EmptyState>
    </section>
  );
}

/** The empty Trust page: how to start, in two sentences. */
export function TrustStart() {
  return (
    <section className="card">
      <EmptyState title="Nothing here yet">
        Quod skips test files whose code did not change since a recorded run, and reuses their result. To start, add <code className="mono">pango07/quod-action</code> to your
        CI; your test files show up here after its first run.
      </EmptyState>
    </section>
  );
}

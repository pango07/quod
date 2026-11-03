"use client";

// Test files per suite, on the Trust page (test-level replay). Files that need
// attention (paused, always runs, learning) come first; the rest (skipping,
// ready to skip) fold under a count. Words come from lib/test-suites.ts.

import Link from "next/link";
import { useState } from "react";
import { Dot, EmptyState } from "@/components/ui";
import type { TestFile, TestSuite } from "@/lib/api/types";
import { formatAgo, formatCount } from "@/lib/format";
import { formatSavedMs, SAVED_EXPLAINED } from "@/lib/test-runs";
import { FILE_LEGEND, fileStateLabel, lastResultText, splitFiles, stateCounts, suiteHeading, suiteLedgerText, suiteTotalsText } from "@/lib/test-suites";

/** Skipping and ready files are listed as they are up to this many; more fold under a count. */
const SHOW_REST_UP_TO = 8;

function FileRow({ f }: { f: TestFile }) {
  const s = fileStateLabel(f);
  const tone = s.tone;
  return (
    <div className="trow cols-plain">
      <span className="cell-main">
        <span className="mono ellipsis" style={{ fontSize: 13 }} title={f.file}>
          {f.file}
        </span>
        {f.project ? <span className="cell-sub">{f.project}</span> : null}
      </span>
      <span>
        <span className="sr-only">State: </span>
        <span className={`chip wrap ${tone === "blue" || tone === "amber" ? tone : ""}`} title={s.help}>
          <Dot tone={tone === "gray" ? "white" : tone} size="sm" />
          {s.label}
        </span>
      </span>
      <span style={{ fontSize: 13 }}>
        <span className="label-sm">Last result </span>
        <span style={f.lastResult === "fail" ? { color: "var(--red)" } : undefined}>{lastResultText(f.lastResult)}</span>
      </span>
      <span style={{ fontSize: 13 }}>
        <span className="label-sm">Skipped </span>
        {f.skipped > 0 ? `${formatCount(f.skipped)}×` : "Not yet"}
      </span>
      <span className="mono" style={{ fontSize: 13 }} title={SAVED_EXPLAINED}>
        <span className="label-sm" style={{ fontFamily: "var(--sans)" }}>
          Time saved{" "}
        </span>
        {f.savedMs ? formatSavedMs(f.savedMs) : "0 s"}
      </span>
    </div>
  );
}

function SuiteCard({ team, suite, asOf, index }: { team: string; suite: TestSuite; asOf: string; index: number }) {
  const { attention, rest } = splitFiles(suite.files);
  const fold = rest.length > SHOW_REST_UP_TO;
  const [open, setOpen] = useState(false);
  const headId = `suite-h-${index}`;
  const restId = `suite-rest-${index}`;
  const restCounts = stateCounts({ learning: 0, always: 0, paused: 0, skipping: rest.filter((f) => f.state === "skipping").length, ready: rest.filter((f) => f.state === "ready").length });
  // The server's totals can count more files than it lists (a long suite): say so rather than imply the list is complete.
  const unlisted = Math.max(0, suite.totals.files - suite.files.length);
  return (
    <section className="card" aria-labelledby={headId} style={{ overflow: "hidden" }}>
      <div className="card-head" style={{ alignItems: "flex-start" }}>
        <div style={{ display: "flex", flexDirection: "column", gap: 4, minWidth: 0 }}>
          <h2 id={headId} className="h2">
            {suiteHeading(suite)}
          </h2>
          <p className="muted" style={{ fontSize: 13, lineHeight: 1.5 }}>
            {suiteTotalsText(suite.totals)}
          </p>
          <p className="faint" style={{ fontSize: 12, lineHeight: 1.5 }}>
            {suiteLedgerText(suite.ledger, (iso) => formatAgo(iso, asOf))}
          </p>
        </div>
        <Link href={`/${team}/${suite.repoSlug}/tests`} className="link-muted" style={{ whiteSpace: "nowrap" }}>
          Test runs
        </Link>
      </div>
      <div className="table-head cols-plain" aria-hidden="true">
        <span>Test file</span>
        <span>State</span>
        <span>Last result</span>
        <span>Skipped</span>
        <span title={SAVED_EXPLAINED}>Time saved</span>
      </div>
      {attention.map((f) => (
        <FileRow key={f.file} f={f} />
      ))}
      {attention.length === 0 && suite.files.length > 0 ? (
        <p className="trow muted" style={{ display: "block", fontSize: 13 }}>
          Nothing needs a look: every test file is skipping or ready to skip.
        </p>
      ) : null}
      {fold ? (
        <>
          <div className="trow" style={{ display: "block", fontSize: 13 }}>
            <button type="button" className="link-btn" aria-expanded={open} aria-controls={restId} onClick={() => setOpen((o) => !o)}>
              {open ? `Hide ${formatCount(rest.length)} more` : `Show ${formatCount(rest.length)} more: ${restCounts}`}
            </button>
          </div>
          {open ? (
            <div id={restId}>
              {rest.map((f) => (
                <FileRow key={f.file} f={f} />
              ))}
            </div>
          ) : null}
        </>
      ) : (
        rest.map((f) => <FileRow key={f.file} f={f} />)
      )}
      {unlisted > 0 ? (
        <p className="trow faint" style={{ display: "block", fontSize: 12 }}>
          {formatCount(unlisted)} more not listed.
        </p>
      ) : null}
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

/** Trust, test-first: one card per suite, then what the states mean. */
export function TestSuitesList({ team, suites, asOf }: { team: string; suites: TestSuite[]; asOf: string }) {
  return (
    <>
      {suites.map((s, i) => (
        <SuiteCard key={`${s.repository}:${s.suite}`} team={team} suite={s} asOf={asOf} index={i} />
      ))}
      <details className="plain-legend card" style={{ borderTop: 0 }}>
        <summary>What the states mean</summary>
        <dl>
          {FILE_LEGEND.map((l) => (
            <div key={l.state}>
              <dt>{l.label}</dt>
              <dd>{l.help}</dd>
            </div>
          ))}
          <div>
            <dt>Time saved</dt>
            <dd>{SAVED_EXPLAINED}</dd>
          </div>
        </dl>
      </details>
    </>
  );
}

"use client";

import { useOptimistic, useState, useTransition } from "react";
import { Dot } from "@/components/ui";
import { setProofsNeeded, type ActionResult } from "@/lib/actions";
import type { TrustEvent } from "@/lib/api/types";
import { formatAgo, formatDuration, formatMinutes } from "@/lib/format";

const CHOICES = [2, 3, 5];

/** Step.dc.html status card: the streak toward trust and the "runs needed" toggle. */
export function ProofsCard({
  team,
  repo,
  stepKey,
  paused,
  have,
  need,
  canEdit,
  children,
}: {
  team: string;
  repo: string;
  stepKey: string;
  paused: boolean;
  have: number;
  need: number;
  canEdit: boolean;
  children?: React.ReactNode;
}) {
  const [optimistic, setOptimistic] = useOptimistic(need);
  const [pending, start] = useTransition();
  const [result, setResult] = useState<ActionResult>(null);
  const streak = Math.min(have, optimistic);
  const pct = Math.round((streak / optimistic) * 100);

  const choose = (n: number) =>
    start(async () => {
      setOptimistic(n);
      setResult(await setProofsNeeded(team, repo, stepKey, n));
    });

  return (
    <section className="card" aria-label="Status" style={{ padding: 20, display: "flex", flexDirection: "column", gap: 16 }}>
      <p style={{ fontSize: 15, lineHeight: 1.6 }}>
        {paused ? (
          <>
            A spot check re-ran this step and got a different result from the same inputs. Quod stopped skipping it. It runs every time until it gives the same result{" "}
            <span style={{ fontWeight: 600 }}>{optimistic} times in a row</span>.
          </>
        ) : (
          <>
            Quod replays this step only after the same inputs give the same result <span style={{ fontWeight: 600 }}>{optimistic} times</span>. Until then it runs every time.
          </>
        )}
      </p>
      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        <div style={{ display: "flex", justifyContent: "space-between", fontSize: 13, color: "var(--muted)" }}>
          <span id="streak-label">{paused ? "Consistent runs since the pause" : "Identical runs so far"}</span>
          <span className="mono" style={{ color: "var(--text)" }}>
            {streak} of {optimistic}
          </span>
        </div>
        <div className="progress" role="progressbar" aria-labelledby="streak-label" aria-valuemin={0} aria-valuemax={optimistic} aria-valuenow={streak}>
          <div style={{ width: `${pct}%` }} />
        </div>
      </div>
      <div className="row-between" style={{ paddingTop: 4 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 10, fontSize: 13, color: "var(--muted)", flexWrap: "wrap" }}>
          <span id="proofs-label">Runs needed to trust it {paused ? "again" : ""}</span>
          <div className="seg" role="group" aria-labelledby="proofs-label">
            {CHOICES.map((n) => (
              <button
                key={n}
                type="button"
                aria-pressed={n === optimistic}
                disabled={!canEdit || pending}
                title={canEdit ? undefined : "Only team admins can change this"}
                onClick={() => n !== optimistic && choose(n)}
              >
                {n}
              </button>
            ))}
          </div>
        </div>
        {children}
      </div>
      {result && !result.ok ? (
        <p role="alert" className="toast err">
          {result.message}
        </p>
      ) : (
        <p role="status" className="sr-only">
          {result?.message ?? ""}
        </p>
      )}
    </section>
  );
}

function describe(e: TrustEvent): { what: string; detail: string; tone: "white" | "amber" | "green" | "blue" | "dim" } {
  switch (e.kind) {
    case "consistent":
      return { what: "Consistent run", detail: `run #${e.run} · same result as #${e.sameAs}`, tone: "white" };
    case "paused":
      return { what: "Paused", detail: `spot check #${e.run} gave a different result`, tone: "amber" };
    case "spot-check-passed":
      return { what: "Spot check passed", detail: `run #${e.run} · identical`, tone: "green" };
    case "replayed":
      return { what: "Replayed", detail: `${e.times} times, saved ${formatMinutes(e.savedMinutes)}`, tone: "blue" };
    case "proven":
      return { what: "Proven", detail: `identical in runs #${e.runs[0]} and #${e.runs[1]}`, tone: "green" };
    case "first-recorded":
      return { what: "First recorded", detail: `run #${e.run} · ${formatDuration(e.seconds)}`, tone: "dim" };
    case "proofs-changed":
      return { what: "Runs needed changed", detail: `${e.from} → ${e.to} by ${e.by}`, tone: "white" };
  }
}

const COLLAPSED = 4;

/** Step.dc.html history card with "Show all / Show less". */
export function History({ events, asOf }: { events: TrustEvent[]; asOf: string }) {
  const [all, setAll] = useState(false);
  const shown = all ? events : events.slice(0, COLLAPSED);
  return (
    <section className="card" aria-labelledby="history-h" style={{ flex: "1 1 360px", padding: 20, display: "flex", flexDirection: "column" }}>
      <div className="row-between" style={{ paddingBottom: 8 }}>
        <h2 id="history-h" className="h2">
          History
        </h2>
        {events.length > COLLAPSED ? (
          <button type="button" className="btn sm" aria-expanded={all} aria-controls="history-list" onClick={() => setAll((a) => !a)}>
            {all ? "Show less" : "Show all"}
          </button>
        ) : null}
      </div>
      <ol id="history-list" style={{ listStyle: "none", margin: 0, padding: 0 }}>
        {shown.map((e, i) => {
          const d = describe(e);
          return (
            <li key={i} className="ev">
              <Dot tone={d.tone === "dim" ? "dim" : d.tone} size="lg" />
              <span style={{ display: "flex", flexDirection: "column", gap: 2 }}>
                <span>{d.what}</span>
                <span className="faint">{d.detail}</span>
              </span>
              <time className="mono faint" dateTime={e.at} style={{ whiteSpace: "nowrap" }}>
                {formatAgo(e.at, asOf)}
              </time>
            </li>
          );
        })}
      </ol>
    </section>
  );
}

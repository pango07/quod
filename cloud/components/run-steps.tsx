"use client";

import Link from "next/link";
import { useEffect, useRef, useState } from "react";
import { Elapsed } from "@/components/live/parts";
import { OutcomeChip, StepDot } from "@/components/ui";
import type { RunJob } from "@/lib/api/types";
import { formatDuration } from "@/lib/format";
import { receiptFor } from "@/lib/trust";

/**
 * Run.dc.html: the steps on the left are buttons; the panel on the right says
 * why the selected one replayed or ran. The selection is kept in the URL (?step=).
 * Arrow keys move between steps.
 */
export function RunSteps({ team, repo, jobs, initial }: { team: string; repo: string; jobs: RunJob[]; initial: number }) {
  const flat = jobs.flatMap((j, ji) => j.steps.map((s, si) => ({ s, ji, si })));
  const [sel, setSel] = useState(Math.min(Math.max(0, initial), flat.length - 1));
  const list = useRef<HTMLDivElement>(null);
  const cur = flat[Math.min(sel, flat.length - 1)];
  const r = receiptFor(cur.s);

  // Steps whose state just changed get a brief highlight (none with prefers-reduced-motion; see CSS).
  const outcomes = flat.map((f) => f.s.outcome).join(",");
  const prev = useRef<string[] | null>(null);
  const [fresh, setFresh] = useState<Set<number>>(new Set());
  useEffect(() => {
    const now = outcomes.split(",");
    if (prev.current) {
      const changed = new Set(now.flatMap((o, i) => (prev.current![i] !== o ? [i] : [])));
      if (changed.size) {
        setFresh(changed);
        const t = setTimeout(() => setFresh(new Set()), 1400);
        prev.current = now;
        return () => clearTimeout(t);
      }
    }
    prev.current = now;
  }, [outcomes]);

  const pick = (i: number, focus = false) => {
    setSel(i);
    const url = new URL(window.location.href);
    url.searchParams.set("step", String(i + 1));
    window.history.replaceState(null, "", url);
    if (focus) list.current?.querySelectorAll<HTMLButtonElement>("button.step-btn")[i]?.focus();
  };

  // Each job's steps start at this index in the flat list.
  const offsets = jobs.map((_, ji) => jobs.slice(0, ji).reduce((a, j) => a + j.steps.length, 0));
  return (
    <div className="split">
      <div ref={list} style={{ flex: "1 1 420px", display: "flex", flexDirection: "column", gap: 16 }}>
        {jobs.map((job, ji) => (
          <section key={job.name} className="card" aria-label={`Steps in ${job.name}`} style={{ overflow: "hidden" }}>
            <h2 className="h2" style={{ padding: "14px 16px" }}>
              {job.name}
            </h2>
            {job.steps.map((s, si) => {
              const i = offsets[ji] + si;
              return (
                <button
                  key={`${ji}-${i}`}
                  type="button"
                  className="step-btn"
                  aria-pressed={i === sel}
                  data-state={s.outcome}
                  data-fresh={fresh.has(i) ? "true" : undefined}
                  aria-controls="why-panel"
                  onClick={() => pick(i)}
                  onKeyDown={(e) => {
                    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                      e.preventDefault();
                      pick((i + (e.key === "ArrowDown" ? 1 : flat.length - 1)) % flat.length, true);
                    }
                  }}
                >
                  <StepDot outcome={s.outcome} />
                  <span className="ellipsis">{s.name}</span>
                  <OutcomeChip outcome={s.outcome} />
                  <span className="mono" style={{ textAlign: "right", color: "var(--muted)", fontSize: 13 }}>
                    {s.outcome === "queued" ? "" : s.outcome === "running" && s.startedAt ? <Elapsed since={s.startedAt} /> : formatDuration(s.seconds)}
                  </span>
                </button>
              );
            })}
          </section>
        ))}
      </div>

      <section
        id="why-panel"
        className="card why-panel"
        aria-labelledby="why-h"
        style={{ flex: "1 1 380px", padding: 20, display: "flex", flexDirection: "column", gap: 14 }}
      >
        <p className="sr-only" aria-live="polite">
          {r.title}: {r.state}.
        </p>
        <div className="row-between" style={{ flexWrap: "nowrap" }}>
          <h2 id="why-h" className="h2 ellipsis" style={{ minWidth: 0 }}>
            {r.title}
          </h2>
          <span className={`chip ${r.tone === "blue" ? "blue" : ""}`}>{r.state}</span>
        </div>
        <p className="muted" style={{ fontSize: 14, lineHeight: 1.6 }}>
          {r.why}
        </p>
        <pre className="well" aria-label="Quod receipt">
          {r.receipt}
        </pre>
        <dl style={{ margin: 0 }}>
          {r.facts.map((f) => (
            <div key={f.k} className="kv">
              <dt>{f.k}</dt>
              <dd style={{ margin: 0 }}>{f.v}</dd>
            </div>
          ))}
        </dl>
        {r.stepKey ? (
          <Link href={`/${team}/${repo}/steps/${r.stepKey}`} className="link-muted">
            See this step’s trust history
          </Link>
        ) : null}
      </section>
    </div>
  );
}

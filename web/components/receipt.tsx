"use client";

import { useEffect, useState } from "react";
import { sleep, useInView, useReducedMotion } from "@/lib/use-in-view";

const LINES: { text: string; head?: boolean; title: string; body: string }[] = [
  {
    text: "QUOD  REPLAYED",
    head: true,
    title: "Replayed, not run.",
    body: "The result came from a proven earlier run. Its output and exit code follow, byte for byte.",
  },
  {
    text: "  proved by run 01JCK3Q9 on 2026-11-14 09:12 UTC",
    title: "Which run proved it.",
    body: "Open that run to see its log, its commit and everything it read.",
  },
  {
    text: "  step 4c1e9a07  manifest 9a07f2d1  2,314 inputs matched",
    title: "What matched.",
    body: "Every file the step read, every directory it listed and every path it looked for and didn't find. All 2,314 were identical.",
  },
  {
    text: "  proven 2 of 2  audited 17 times  0 divergences",
    title: "Earned trust.",
    body: "This step replays only after identical proofs, and a share of replays keep re-running as audits.",
  },
  {
    text: "  recorded 4m12s  replay 0.4s  saved 4m11s",
    title: "Time back.",
    body: "The recorded run took 4m12s. The replay took 0.4s.",
  },
];

export function Receipt() {
  const [ref, inView] = useInView<HTMLDivElement>(0.5);
  const reduce = useReducedMotion();
  const [on, setOn] = useState(0);
  const [hold, setHold] = useState(false);

  useEffect(() => {
    if (!inView || hold || reduce) return;
    const ctl = new AbortController();
    (async () => {
      for (let i = 0; ; i = (i + 1) % LINES.length) {
        setOn(i);
        await sleep(2600, ctl.signal);
      }
    })().catch(() => {});
    return () => ctl.abort();
  }, [inView, hold, reduce]);

  const line = LINES[on];
  return (
    <div className="receipt" ref={ref} onMouseLeave={() => setHold(false)}>
      <div className="receipt-lines" role="list" aria-label="A Quod receipt">
        {LINES.map((l, i) => (
          <button
            key={l.text}
            role="listitem"
            type="button"
            className={`ln ${l.head ? "head" : ""} ${i === on ? "on" : ""}`}
            onMouseEnter={() => {
              setHold(true);
              setOn(i);
            }}
            onFocus={() => {
              setHold(true);
              setOn(i);
            }}
            onClick={() => setOn(i)}
          >
            {l.text}
          </button>
        ))}
      </div>
      <p className="explain" aria-live="polite">
        <b>{line.title}</b> {line.body}
      </p>
    </div>
  );
}

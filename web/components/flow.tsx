"use client";

import { useEffect, useState } from "react";
import { useInView, useReducedMotion } from "@/lib/use-in-view";

const STEPS = [
  { title: "Record", body: "Quod runs your tests and records the inputs used by each file, including shared configuration and loaded dependencies.", detail: "Example: calc.test.ts reads calc.ts. Editing that source file makes this test file run again." },
  { title: "Verify", body: "A fresh result needs confirmation before reuse. Quod compares test outcomes and checks the recorded inputs on later runs.", detail: "Example: another execution agrees on the per-test states. The record can become eligible for reuse." },
  { title: "Reuse", body: "Eligible files with unchanged inputs reuse their results. Changed and ineligible files run. Audits keep checking reused results.", detail: "Example: editing an unread README leaves the recorded inputs unchanged. An eligible result can be reused, unless an audit is due." },
];

export function Flow() {
  const [ref, inView] = useInView<HTMLDivElement>();
  const reduced = useReducedMotion();
  const [on, setOn] = useState(0);
  const [paused, setPaused] = useState(false);
  useEffect(() => {
    if (!inView || reduced || paused) return;
    const timer = setInterval(() => setOn(value => (value + 1) % STEPS.length), 4500);
    return () => clearInterval(timer);
  }, [inView, reduced, paused]);

  return (
    <div ref={ref} className="flow-demo">
      <ol className="flow">
        {STEPS.map((step, i) => (
          <li key={step.title} className={i === on ? "lit" : ""}>
            <button type="button" aria-pressed={i === on} onClick={() => { setPaused(true); setOn(i); }}>
              <span className="n">0{i + 1}</span><span className="flow-title">{step.title}</span>
            </button>
            <p>{step.body}</p>
          </li>
        ))}
      </ol>
      <div className="flow-detail">
        <p aria-live={paused || reduced ? "polite" : "off"}>{STEPS[on].detail}</p>
        {!reduced && <button type="button" className="demo-control" onClick={() => setPaused(value => !value)}>{paused ? "Play stages" : "Pause stages"}</button>}
      </div>
    </div>
  );
}

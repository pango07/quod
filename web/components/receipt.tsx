"use client";

import { useState } from "react";
import { benchmark } from "@/lib/benchmark";

const LINES = [
  { label: "12 commits · cold start included", title: "The whole window.", body: "The comparison includes the initial recording, confirmations, audits and Action overhead. Separate warm probes are excluded." },
  { label: "1,766s plain → 968s with Quod", title: "45.2% less runner time.", body: "798 runner-seconds saved across the paired jobs, including installs, builds, tests, setup and post steps." },
  { label: "36 min plain → 21 min with Quod", title: "41.7% lower estimated usage charges.", body: "Each job is rounded up to a minute before summing. At the same runner rate, this saves 15 metered minutes. The actual public jobs were free." },
  { label: "0 wrong observed on every commit", title: "Per-test agreement.", body: "Names, pass/fail states, counts and job conclusions matched the paired plain jobs. This is evidence for this window, not a universal guarantee." },
];

export function Receipt() {
  const [on, setOn] = useState(0);
  return (
    <div className="receipt">
      <p className="demo-note">Historical engine: these results predate the metadata correctness fix. <a href={benchmark.currentReport}>See the corrected-engine results ↗</a></p>
      <div className="receipt-lines" role="group" aria-label="Explore the benchmark evidence">
        {LINES.map((line, i) => <button key={line.label} type="button" aria-pressed={i === on} className={`ln ${i === on ? "on" : ""}`} onClick={() => setOn(i)}>{line.label}</button>)}
      </div>
      <p className="explain" aria-live="polite"><b>{LINES[on].title}</b> {LINES[on].body}</p>
      <p className="demo-note"><a href={benchmark.report}>Measured evidence and pinned code ↗</a></p>
    </div>
  );
}

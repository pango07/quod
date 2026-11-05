"use client";

import { useState } from "react";
import { benchmark, costSaved, duration, timeSaved } from "@/lib/benchmark";

export function CiJob() {
  const [metric, setMetric] = useState<"time" | "cost">("time");
  const isTime = metric === "time";
  const plain = isTime ? benchmark.plainSeconds : benchmark.plainMinutes;
  const quod = isTime ? benchmark.quodSeconds : benchmark.quodMinutes;
  const format = isTime ? duration : (minutes: number) => `${minutes} min`;
  return (
    <figure className="job benchmark-job" aria-labelledby="job-caption">
      <figcaption className="job-head" id="job-caption">
        <span className="job-title">convex-helpers · 12 commits</span>
        <span className="job-label">Measured on GitHub Actions</span>
      </figcaption>
      <div className="job-controls seg" role="group" aria-label="Compare benchmark metrics">
        <button type="button" aria-pressed={isTime} onClick={() => setMetric("time")}>Runner time</button>
        <button type="button" aria-pressed={!isTime} onClick={() => setMetric("cost")}>Runner cost</button>
      </div>
      <div className="comparison" aria-live="polite">
        <p className="job-label">{isTime ? "Total runner time, including the cold start" : "Rounded runner-minutes, including the cold start"}</p>
        <div className="comparison-row">
          <div><span>Plain CI</span><strong>{format(plain)}</strong></div>
          <div className="comparison-track" aria-hidden="true"><span style={{ width: "100%" }} /></div>
        </div>
        <div className="comparison-row quod-row">
          <div><span>With Quod</span><strong>{format(quod)}</strong></div>
          <div className="comparison-track" aria-hidden="true">
            <span style={{ width: `${quod / plain * 100}%` }} />
          </div>
        </div>
        <p className="job-label">{isTime ? "Same commits. Same test outcomes. Less repeated work." : "Estimated usage charges at the same runner rate. These public jobs were free."}</p>
      </div>
      <div className="job-foot">
        <span><b>{isTime ? timeSaved : costSaved}%</b> {isTime ? `less runner time · ${duration(plain - quod)} saved` : "lower estimated runner charges"}</span>
        <a href={benchmark.run}>View the runs ↗</a>
      </div>
    </figure>
  );
}

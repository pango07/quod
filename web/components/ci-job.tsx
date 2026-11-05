import { benchmark, duration } from "@/lib/benchmark";

/** Actual cumulative runner time, not an animation of hypothetical jobs. */
export function CiJob() {
  return (
    <figure className="job benchmark-job" aria-labelledby="job-caption">
      <figcaption className="job-head" id="job-caption">
        <span className="job-title">convex-helpers · 12 commits</span>
        <span className="job-label">Measured on GitHub Actions</span>
      </figcaption>
      <div className="comparison">
        <p className="job-label">Total runner time, including the cold start</p>
        <div className="comparison-row">
          <div><span>Plain CI</span><strong>{duration(benchmark.plainSeconds)}</strong></div>
          <div className="comparison-track" aria-hidden="true"><span style={{ width: "100%" }} /></div>
        </div>
        <div className="comparison-row quod-row">
          <div><span>With Quod</span><strong>{duration(benchmark.quodSeconds)}</strong></div>
          <div className="comparison-track" aria-hidden="true">
            <span style={{ width: `${benchmark.quodSeconds / benchmark.plainSeconds * 100}%` }} />
          </div>
        </div>
        <p className="job-label">Same commits. Same test outcomes. Less repeated work.</p>
      </div>
      <div className="job-foot">
        <span><b>{duration(benchmark.plainSeconds - benchmark.quodSeconds)}</b> of runner time saved</span>
        <a href={benchmark.run}>View the runs ↗</a>
      </div>
    </figure>
  );
}

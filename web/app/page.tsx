import Link from "next/link";
import { CiJob } from "@/components/ci-job";
import { Flow } from "@/components/flow";
import { JoinForm } from "@/components/join-form";
import { SiteFooter, SiteNav } from "@/components/site-nav";
import { WorkflowDiff } from "@/components/workflow-diff";
import { benchmark, costSaved, timeSaved } from "@/lib/benchmark";

const FAQ: [string, string][] = [
  ["What does Quod actually skip?",
    "Eligible Vitest test files whose recorded inputs are unchanged and whose results have been verified. Files with changed inputs still run. Dependency caching can also avoid repeat installs; experimental command replay covers selected npm scripts."],
  ["Will my CI always be 40–50% cheaper?",
    "That is the range we measured for runner time and estimated runner charges on convex-helpers, including its cold start. Your savings depend on what changes, test duration and how often results can be reused. First recordings, confirmations and audits add work. Individual jobs can be slower."],
  ["What is supported?",
    "The benchmark covers one-shot Vitest on GitHub-hosted Linux x64, using Node 24.18.0 and Vitest 5.0.0, in a monorepo with multiple test projects. We are not claiming these savings for other test frameworks, operating systems or runner architectures. Watch mode, browser mode and other unsupported command modes run normally."],
  ["What does zero wrong replays mean?",
    "On every commit in this benchmark, Quod matched the plain job’s per-test names and pass/fail states, test counts and job conclusion. That is zero observed differences in this window, not proof that every possible test behavior is covered. Quod also audits reused results and stops reusing records that disagree."],
  ["Why are time savings and cost savings different?",
    "We measured runner-seconds for time. For estimated usage cost, we rounded each job up to a full minute, as GitHub does, then compared the totals at the same per-minute rate. The public benchmark jobs were free. These are two views of the same saved work; the percentages do not add together."],
  ["Are these the released Action’s default results?",
    "The reported window uses an experimental Action revision with lineage witnesses and a configuration-input proof enabled. The report pins the code, settings and every job. The released v2 Action has different defaults; these numbers describe the benchmark configuration."],
];

export default function Home() {
  return (
    <>
      <SiteNav />
      <main id="top">
        <section className="hero">
          <div className="hero-txt">
            <p className="eyebrow">Vitest CI, with less repeated work</p>
            <h1>Faster checks. Fewer runner minutes.</h1>
            <p className="lede">
              Quod records the inputs each test file uses. When those inputs stay the same and its result is verified,
              Quod reuses that result. Changed files still run.
            </p>
            <JoinForm source="hero" />
            <a className="text-link" href="#benchmarks">See the measured savings ↓</a>
          </div>
          <CiJob />
        </section>
        <section className="sec" id="benchmarks">
          <div className="sec-head">
            <p className="eyebrow">Observed results · November 5, 2026</p>
            <h2>Real jobs. Measured savings.</h2>
            <p className="lede">
              We compared Quod with plain CI across the same 12 commits of convex-helpers.
              The window includes the cold start, installs, builds, tests and action overhead.
            </p>
          </div>
          <div className="proof">
            <div><span className="big">{timeSaved}%</span><p>less total runner time</p><p>1,766 → 968 runner-seconds</p></div>
            <div><span className="big">{costSaved}%</span><p>lower estimated runner charges</p><p>36 → 21 rounded runner-minutes</p></div>
            <div><span className="big">floor + {benchmark.bestHitSeconds - benchmark.bestFloorSeconds}s</span><p>best separate warm-hit job</p><p>{benchmark.bestHitSeconds}s total, against an {benchmark.bestFloorSeconds}s empty job</p></div>
          </div>
          <div className="benchmark-notes">
            <p><strong>0 wrong replays observed.</strong> Every commit matched the paired plain job&rsquo;s test outcomes.</p>
            <p>Three separate warm-hit probes had a median job duration of {benchmark.medianHitSeconds}s. They are excluded from the 12-commit totals.</p>
            <p className="foot-note">
              Experimental settings, one repository. Savings vary. Cost is estimated from rounded minutes at the same
              runner rate; these public jobs cost $0. The report includes the code pins, settings and runner images.
            </p>
            <p className="evidence-links"><a href={benchmark.report}>Read the benchmark report ↗</a><a href={benchmark.run}>Inspect the GitHub jobs ↗</a></p>
          </div>
        </section>
        <section className="sec" id="how">
          <div className="sec-head"><h2>Record. Verify. Reuse.</h2></div>
          <Flow />
          <p className="more"><Link href="/under-the-hood">How inputs, confirmations and audits work →</Link></p>
        </section>
        <section className="sec split" id="setup">
          <div className="sec-head">
            <h2>Add one Action step.</h2>
            <p className="lede">Keep your GitHub runner, install command and Vitest tests. Add Quod after Node setup.</p>
            <p className="foot-note">This shows the released Action. The benchmark report documents the experimental settings behind the measured savings.</p>
            <a className="text-link" href="https://github.com/pango07/quod-action/tree/v2">Read the setup guide ↗</a>
          </div>
          <WorkflowDiff />
        </section>
        <section className="sec split" id="scope">
          <div className="sec-head">
            <h2>Built for supported Vitest jobs.</h2>
            <p className="lede">Start with Linux x64 on GitHub Actions. Monorepos and multiple Vitest projects are part of the measured workload.</p>
          </div>
          <div className="sec-head">
            <ul className="points">
              <li>Fresh records need verification before reuse.</li>
              <li>Changed inputs and ineligible test files execute again.</li>
              <li>Audits compare reused results with another execution.</li>
              <li>Cold starts and shared configuration changes can cost more.</li>
            </ul>
            <a className="text-link" href={benchmark.support}>See the tested setup and current limits ↗</a>
          </div>
        </section>
        <section className="sec" id="faq">
          <div className="sec-head"><h2>Questions</h2></div>
          <div className="faq">{FAQ.map(([q, a]) => <details key={q}><summary>{q}</summary><p>{a}</p></details>)}</div>
        </section>
        <section className="cta" id="join">
          <h2>Make less work of CI.</h2>
          <p className="lede">Join the waitlist to try Quod with your team&rsquo;s Vitest workflow.</p>
          <JoinForm source="footer" />
        </section>
      </main>
      <SiteFooter />
    </>
  );
}

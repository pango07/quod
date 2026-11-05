import type { Metadata } from "next";
import Link from "next/link";
import { Flow } from "@/components/flow";
import { SiteFooter, SiteNav } from "@/components/site-nav";
import { benchmark } from "@/lib/benchmark";

export const metadata: Metadata = {
  title: "How Quod reuses Vitest results",
  description: "What Quod records, how results become eligible for reuse, and what the convex-helpers benchmark establishes.",
};

export default function UnderTheHood() {
  return (
    <>
      <SiteNav />
      <main>
        <section className="hero hood-hero">
          <div className="hero-txt">
            <p className="eyebrow">How it works</p>
            <h1>Inside Quod&rsquo;s Vitest replay.</h1>
            <p className="lede">Quod saves verified test results and checks whether the inputs behind them changed. The unit of reuse is a test file within its Vitest project.</p>
          </div>
        </section>
        <section className="sec" id="sequence">
          <div className="sec-head"><h2>Record. Verify. Reuse.</h2></div>
          <Flow />
        </section>
        <section className="sec" id="inputs">
          <div className="sec-head"><h2>What counts as an input?</h2></div>
          <div className="reading">
            <p>The recorder follows loaded modules and observed file, directory and environment reads. Shared configuration and tool versions also matter. A configuration change can affect many test files at once.</p>
            <p>A missing file can be an input too. If a test checks for a configuration file that does not exist, creating it later can change the result. Directory listings matter when a tool discovers files.</p>
            <p>Quod stores those observations with the test outcome. On a later run, it compares their current states. A changed recorded input makes the affected file run again.</p>
            <p>The experimental benchmark also uses a bounded proof for configuration code that reads dependency names. It keeps value dependencies when code uses those values. It does not simply ignore edits to package.json.</p>
          </div>
        </section>
        <section className="sec" id="verification">
          <div className="sec-head"><h2>How a result earns reuse.</h2></div>
          <div className="reading">
            <h3>Confirmation</h3>
            <p>A fresh record normally waits for another agreeing execution. When its recorded inputs still match, that confirmation can run without the recorder, allowing Quod to compare per-test outcomes with plain Vitest.</p>
            <h3>Lineage witnesses in the benchmark</h3>
            <p>The experimental policy can promote a new passing record when it replaces a plain-trusted record, observes the same input set and produces the same per-test states. Network use, near-timeout tests and quarantined records prevent that shortcut. A sample then runs plain for an audit. This policy is switchable and is not the released Action&rsquo;s default.</p>
            <h3>Audits and disagreements</h3>
            <p>Audits execute selected files again and compare outcomes. A disagreement stops reuse for that file. A failed lineage audit also revokes trust for records promoted in the same batch.</p>
            <p><strong>These checks are evidence, not a universal correctness guarantee.</strong> An audit may discover a problem after a replay. Recorder coverage, timing-sensitive tests and unsupported behavior still need careful validation. The support notes describe the current boundaries and open observation risks.</p>
            <p><a href={benchmark.support}>Read the current support and correctness boundaries ↗</a></p>
          </div>
        </section>
        <section className="sec" id="cost">
          <div className="sec-head"><h2>Count the whole job.</h2></div>
          <div className="reading">
            <p>The benchmark compares the same 12 commits with and without Quod. Total runner time includes installs, builds, tests, setup and post steps. The cold recording, confirmations and audits stay in the measured window.</p>
            <p>Runner time fell from 1,766 to 968 seconds: 45.2% less. Rounding each job up to a full minute gives 36 minutes plain and 21 with Quod: 41.7% less metered usage. At the same runner rate, that is the estimated reduction in usage charges. The actual public jobs were free.</p>
            <p>The estimate follows <a href="https://docs.github.com/en/billing/reference/actions-runner-pricing">GitHub&rsquo;s per-job minute rounding</a>. It covers runner usage, before included allowances, storage charges or Quod pricing.</p>
            <p>All per-commit test outcomes matched. That is zero observed wrong replays in this window, not a claim about every repository. The experiment used Linux x64, Node 24.18.0, Vitest 5.0.0 and explicit experimental settings.</p>
            <p>Warm hits were measured separately at the final unchanged commit. The best job took 14 seconds against an 11-second empty job: <strong>floor + 3 seconds</strong>. Three warm probes had a median duration of 20 seconds. Their times do not reduce the 12-commit total.</p>
            <p className="evidence-links"><a href={benchmark.report}>Measurements, settings and code pins ↗</a><a href={benchmark.run}>Original GitHub runs ↗</a></p>
          </div>
        </section>
        <section className="sec" id="scope">
          <div className="sec-head"><h2>Where to start.</h2></div>
          <div className="reading">
            <p>Use the GitHub Action with supported one-shot Vitest jobs on Linux x64. Monorepos and multiple test projects are part of the measured workload. Add the Action after Node setup and keep your install and test commands.</p>
            <p>First recordings can be slower. Dependency changes, shared configuration changes, confirmations and audits all reduce reuse. Watch mode, browser mode and other unsupported command modes execute normally. These measurements do not establish savings for Windows, macOS, ARM runners or other test frameworks.</p>
            <p>Quod also has a separate sandbox engine and experiments for other languages. Those results are separate from the Vitest Action benchmark shown here.</p>
            <p><Link href="/#setup">See the Action setup →</Link></p>
          </div>
        </section>
      </main>
      <SiteFooter />
    </>
  );
}

// Server parts of the plain trust pages (cloud/trust-plain; lib/ui-version.ts
// switches them on). The classic pages render as before when it is off.

import Link from "next/link";
import { LiveRefresh } from "@/components/live/parts";
import { ProtectedBranchHelp } from "@/components/protected-branch-help";
import { SectionUnavailable } from "@/components/section";
import { TestRunsTable } from "@/components/test-runs";
import { TestSuitesList, TrustStart } from "@/components/test-suites";
import { ProtectionNote, TrustList } from "@/components/trust-plain";
import { Dot } from "@/components/ui";
import { api } from "@/lib/api";
import { section } from "@/lib/api/section";
import type { StepSummary, Team } from "@/lib/api/types";
import { RECENT_RUNS, recentRunDetails } from "@/lib/recent-runs";
import { trustSections } from "@/lib/test-suites";
import { groupSteps, recentByKey } from "@/lib/trust-plain";

/** /[team]/trust, plain: test files first (test-level replay), then steps when there are any. */
export async function PlainTrust({ team }: { team: Team }) {
  const [suitesS, testRunsS, found, runs] = await Promise.all([
    section(api().listTestSuites(team.slug)),
    section(api().listTestRuns(team.slug, { limit: 10 })),
    section(api().listSteps(team.slug)),
    recentRunDetails(team.slug),
  ]);
  const suites = suitesS.ok ? (suitesS.value ?? []) : [];
  const testRuns = testRunsS.ok ? (testRunsS.value ?? []) : [];
  const groups = groupSteps(found.ok ? found.value : [], recentByKey(runs));
  const show = trustSections({ suites, testRuns: testRuns.length, steps: groups.length });
  const paused = groups.filter((g) => g.current.state.kind === "paused");
  const asOf = new Date().toISOString();
  // One "unavailable" card when quod-server is down, not one per source.
  const down = !suitesS.ok ? suitesS : !testRunsS.ok ? testRunsS : !found.ok ? found : null;
  const tests = show.suites || show.testRuns;
  return (
    <div className="page">
      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        <h1 className="h1">Trust</h1>
        <p className="muted" style={{ maxWidth: "72ch", lineHeight: 1.6 }}>
          Quod skips a test file when nothing it depends on has changed since a recorded run on your default branch, and reuses that result. Each file learns first, then it is
          ready to skip. A few skipped files are re-run in the background to check; if one gives a different result, that file is paused and runs every time.
        </p>
      </div>
      <LiveRefresh scope={{ kind: "team" }} types={["test.run"]} />
      {down ? <SectionUnavailable s={down} what={down === found ? "Steps" : "Test files"} /> : null}
      {show.suites ? <TestSuitesList team={team.slug} suites={suites} asOf={asOf} /> : null}
      {show.testRuns ? (
        <section className="card" aria-labelledby="trust-tests-h" style={{ overflow: "hidden" }}>
          <div className="card-head">
            <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
              <h2 id="trust-tests-h" className="h2">
                Recent test runs
              </h2>
              <p className="muted" style={{ fontSize: 13, lineHeight: 1.5, maxWidth: "72ch" }}>
                Each test file&apos;s state shows here once a run on your default branch records it. Until then, these are the latest runs.
              </p>
            </div>
          </div>
          <TestRunsTable team={team.slug} runs={testRuns} asOf={asOf} showRepo />
        </section>
      ) : null}
      {show.empty && !down ? <TrustStart /> : null}
      {show.steps && tests ? (
        <h2 className="h2" style={{ marginTop: 8 }}>
          Steps
        </h2>
      ) : null}
      {paused.map((g) => (
        <Link key={g.id} href={`/${team.slug}/${g.repoSlug}/steps/${g.current.step.key}#diff`} className="notice">
          <Dot tone="amber" />
          <span style={{ display: "flex", flexDirection: "column", gap: 2 }}>
            <span style={{ fontWeight: 500 }}>
              {g.repoSlug} · {g.job ? `${g.job} / ` : ""}
              <span className={g.untitled ? "" : "mono"}>{g.title}</span>: paused
            </span>
            <span className="soft">A re-check got different output from the same inputs. It runs every time until it matches again. Open it to see what differed.</span>
          </span>
        </Link>
      ))}
      {/* Step-level trust: only when there are steps (an empty step table says nothing). */}
      {show.steps ? <TrustList team={team.slug} groups={groups} asOf={asOf} showRepo recentRuns={RECENT_RUNS} /> : null}
      <ProtectionNote>
        <ProtectedBranchHelp bare />
      </ProtectionNote>
    </div>
  );
}

/** /[team]/[repo]/steps, plain. */
export async function PlainRepoSteps({ team, repo, steps }: { team: Team; repo: string; steps: StepSummary[] }) {
  const runs = await recentRunDetails(team.slug, { repo });
  const groups = groupSteps(steps, recentByKey(runs));
  return (
    <div className="page">
      <h1 className="sr-only">Steps of {repo}</h1>
      <TrustList team={team.slug} groups={groups} asOf={new Date().toISOString()} recentRuns={RECENT_RUNS} />
    </div>
  );
}
